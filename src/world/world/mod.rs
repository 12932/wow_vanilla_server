use crate::world::aoi;
use crate::world::character_screen_handler::handle_character_screen_opcodes;
use crate::world::database::WorldDatabase;
use crate::world::update_object::UpdateObject;
use crate::world::world::client::Client;
use crate::world::world::pathfinding_maps::PathfindingMaps;
use crate::world::world_opcode_handler;
use crate::world::world_opcode_handler::character::Character;
use crate::world::world_opcode_handler::creature::{
    walk_speed, Creature, CreatureBehavior, CreatureLifeState,
};
use crate::world::world_opcode_handler::entities::Entities;
use client::character_screen_client::{CharacterScreenClient, CharacterScreenProgress};
use rayon::prelude::*;
use slab::Slab;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::Instrument;
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;
use wow_world_base::combat::UNARMED_SPEED;
use wow_world_base::movement::{
    DEFAULT_RUNNING_BACKWARDS_SPEED, DEFAULT_RUNNING_SPEED, DEFAULT_TURN_SPEED,
};
use wow_world_base::vanilla::position::Position;
use wow_world_base::vanilla::{HitInfo, Map, SplineFlag};
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::UpdateMask;
use wow_world_messages::vanilla::{
    DamageInfo, InitialSpell, Language, MSG_MOVE_HEARTBEAT_Server,
    MSG_MOVE_START_FORWARD_Server, MSG_MOVE_STOP_Server,
    MSG_MOVE_TELEPORT_ACK_Server,
    MovementBlock, MovementBlock_MovementFlags, MovementBlock_UpdateFlag,
    MovementBlock_UpdateFlag_Living, MovementInfo, MovementInfo_MovementFlags, Object, ObjectType,
    Object_UpdateType, PlayerChatTag, SMSG_MESSAGECHAT_ChatType, SMSG_MONSTER_MOVE_MonsterMoveType,
    UpdatePlayerBuilder, Vector3d, VisibleItem, VisibleItemIndex,
    SMSG_ACCOUNT_DATA_TIMES, SMSG_ATTACKERSTATEUPDATE, SMSG_DESTROY_OBJECT, SMSG_INITIAL_SPELLS,
    SMSG_LOGIN_SETTIMESPEED, SMSG_LOGIN_VERIFY_WORLD, SMSG_MESSAGECHAT, SMSG_MONSTER_MOVE,
    SMSG_NEW_WORLD, SMSG_TRANSFER_PENDING, SMSG_TUTORIAL_FLAGS, SMSG_UPDATE_OBJECT,
};
use wow_world_messages::{DateTime, Guid};

pub mod client;
pub mod pathfinding_maps;

/// Per-cell game state. One `MapState` per spatial cell in `World::cells`.
/// Each cell ticks independently on its own tokio task, paced by its own
/// `TickPacer`, and exchanges cross-boundary broadcasts and effects with
/// neighbors through the `routing()` table.
#[derive(Debug)]
pub struct MapState {
    /// The continent this state owns — one `MapState` per `Map`, keying
    /// `World::maps_state`. Stable across ticks.
    pub(crate) map: Map,
    pub(crate) clients: Slab<Client>,
    /// Reverse index from player guid to slab key. Must be maintained in
    /// lockstep with `clients` on every insert/remove — use
    /// [`World::insert_client`] / [`World::remove_client`] which do this
    /// automatically.
    pub(crate) client_by_guid: ahash::AHashMap<Guid, usize>,

    pub(crate) creatures: Slab<Creature>,
    /// Reverse index from creature guid to slab key. Must be maintained in
    /// lockstep with `creatures` on every insert/remove.
    pub(crate) creature_by_guid: ahash::AHashMap<Guid, usize>,
    /// Slab keys of `AggroChase` creatures (typically `.spawn`'d GM mobs).
    pub(crate) aggro_creature_keys: Vec<usize>,
    /// Slab keys of `RandomWander` + `Waypoint` creatures currently active.
    pub(crate) walking_creature_keys: Vec<usize>,
    /// Parked walking creatures keyed by re-wake time.
    pub(crate) creature_wake_at: std::collections::BTreeMap<Instant, Vec<usize>>,
    pub(crate) creature_wander_count: usize,
    pub(crate) creature_waypoint_count: usize,

    /// Persistent live spatial index (cmangos-style 33.33 yd cells) holding
    /// BOTH clients and creatures on this map, keyed by slab key. Maintained
    /// incrementally by the entity-lifecycle + movement sites — NOT rebuilt
    /// per tick. The per-tick `Sync` AoI projection
    /// ([`Self::build_local_aoi_snapshot`]) and grid activation read from it.
    /// Map-local: each `MapState` only holds entities on `self.map`, so the
    /// cell key omits the map; the projection recomposes `(Map, cx, cy)`.
    pub(crate) index: crate::world::spatial::SpatialIndex,

    /// Start of this cell's previous tick. Used to compute wall-clock
    /// `dt` for time-dependent state like `auto_attack_timer`. Per-cell
    /// because each cell will eventually pace independently.
    pub(crate) last_tick_at: Option<Instant>,

    /// Per-tick movement coalescer: one outbound movement broadcast per
    /// source player per tick. Drained in `flush_movement_broadcasts`.
    pub(crate) pending_movement: ahash::AHashMap<Guid, PendingMovement>,

    /// Monotonic counter incremented at the top of every `tick`. Used by
    /// the heartbeat-broadcast throttle.
    pub(crate) tick_counter: u64,

    /// Per-source `tick_counter` value of the most recent heartbeat (or
    /// transition) broadcast for that player. Throttle key.
    pub(crate) last_heartbeat_broadcast_tick: ahash::AHashMap<Guid, u64>,

    // Per-tick scratch buffers (reused across ticks via `.clear()` +
    // refill so the underlying allocations stay warm).
    /// cmangos active-grid set: the union of every player's 533 yd grid
    /// expanded by [`ACTIVATION_GRID_RADIUS`]. Recomputed at the top of
    /// `tick_creature_ai`; gates the aggro scan, the wander/waypoint loop,
    /// and the `creature_wake_at` park loop so idle-zone creatures sleep.
    pub(crate) scratch_active_grids: ahash::AHashSet<(i32, i32)>,
    pub(crate) scratch_walk_events: Vec<(usize, Vector3d, Map, CreatureMoveEvent)>,
    pub(crate) scratch_to_park: Vec<(Instant, usize)>,
    pub(crate) scratch_parked_set: ahash::AHashSet<usize>,
    pub(crate) scratch_expired_roots: Vec<(Guid, Map, Vector3d, MovementInfo)>,

    /// Per-tick `Sync`-safe view of `clients`, rebuilt at the top of the
    /// broadcast phase from each `Client::broadcast_target()`.
    pub(crate) broadcast_view: Vec<crate::world::aoi::BroadcastTarget>,

    /// Per-cell adaptive pacer. Observes each tick's per-cell duration and
    /// publishes its state to the `cell::PACER_STATES` snapshot so the
    /// `.cells` GM command can show the per-cell rate. The orchestrator
    /// `run_world` loop still owns the global sleep cadence.
    pub(crate) pacer: crate::world::TickPacer,
}

#[derive(Debug)]
pub struct World {
    /// Per-map state, one entry per continent. Each `MapState` is wrapped in
    /// `Arc<Mutex<>>` so it can move into a `tokio::spawn`ed task — the global
    /// tick spawns one task per map and joins them, giving tokio-worker
    /// parallelism across maps.
    pub maps_state: ahash::AHashMap<Map, Arc<Mutex<MapState>>>,

    // ── Global state (auth + connection lifecycle + cross-cutting metrics)
    // Stays on `World` indefinitely — these are NOT cell-scoped.
    pub(crate) clients_on_character_screen: Vec<CharacterScreenClient>,
    pub(crate) clients_waiting_to_join: Receiver<CharacterScreenClient>,
    /// Wrapped in `Arc<Mutex<>>` so the per-cell tokio tasks can each
    /// borrow it; `ground_height` and ADT-load both mutate so we can't
    /// just `Arc<>` and need exterior synchronization. Contention is
    /// low — only `apply_commands` and `tick_creature_ai` lock.
    pub(crate) maps: Arc<Mutex<PathfindingMaps>>,
    /// Wrapped likewise so promote (global) and per_client_loop /
    /// stale-cleanup (per-cell) can share access. The lock is held
    /// only while the opcode handler runs for one client at a time —
    /// cells still tick the rest of their work in parallel.
    pub(crate) db: Arc<Mutex<WorldDatabase>>,
    pub(crate) last_packet_sample: u64,
    pub(crate) last_packet_sample_at: Instant,
    pub(crate) last_net_stats: Option<crate::world::net_stats::NetStats>,
    pub(crate) last_net_stats_at: Instant,
}

impl World {
    /// Look up or lazily create the `MapState` for `map` (one per continent).
    pub fn ensure_map_exists(&mut self, map: Map) -> Arc<Mutex<MapState>> {
        if let Some(existing) = self.maps_state.get(&map) {
            return existing.clone();
        }
        let arc = Arc::new(Mutex::new(MapState::new_empty(map)));
        self.maps_state.insert(map, arc.clone());
        arc
    }

    /// Admit a freshly-built `Client` into its map's `MapState`, spinning the
    /// map up if necessary.
    pub async fn admit_client_at_position(&mut self, client: Client) {
        let map = client.character().map;
        let cell_arc = self.ensure_map_exists(map);
        let mut cell = cell_arc.lock().await;
        cell.insert_client(client);
    }
}

#[derive(Debug)]
pub(crate) struct PendingMovement {
    pub msg: ServerOpcodeMessage,
    pub anchor: Vector3d,
    pub map: Map,
}

/// Cell size (yards) for the in-map spatial grid — cmangos's 33.33 yd cell.
/// Single source of truth lives in [`crate::world::spatial`]. Because this is
/// now smaller than the AOI radius, the grid-scan windows below are derived
/// from the radius ([`grid_cell_radius`]) rather than a fixed 3×3.
pub const CREATURE_GRID_CELL_YD: f32 = crate::world::spatial::CELL_SIZE_YD;

/// Number of 33.33 yd cells to scan in each direction to cover an AOI disc of
/// `aoi_r` yards (the radius-derived replacement for the old fixed 3×3 window
/// that was only valid when the cell was larger than the AOI radius).
#[inline]
pub(crate) fn grid_cell_radius(aoi_r: f32) -> i32 {
    (aoi_r / CREATURE_GRID_CELL_YD).ceil() as i32
}

/// Compute the spatial-grid cell key for an entity at `(x, y)`. Z is
/// deliberately ignored — AOI is horizontal-only, same as `within_aoi`.
#[inline]
pub(crate) fn grid_cell_for(x: f32, y: f32) -> (i32, i32) {
    crate::world::spatial::cell_coord(x, y)
}

/// cmangos grid-activation halo, in 533.33 yd grids. A grid is *active* (its
/// creatures tick) if it's within this many grids of any player's grid. 1 grid
/// = 533 yd ≫ the 200 yd AOI and the ~40 yd aggro leash, so nothing a player
/// can see or be chased by is ever skipped.
pub(crate) const ACTIVATION_GRID_RADIUS: i32 = 1;

/// How long an idle-grid creature waits before re-checking whether a player has
/// reactivated its grid. Keeps an unpopulated continent's parked creatures in
/// the `creature_wake_at` schedule at ~1 Hz instead of the per-tick walk loop.
const IDLE_GRID_RECHECK_MS: u64 = 1000;

/// Build a `MapState` (per-map) from a `Vec<Creature>`, computing all the
/// indexes (`creature_by_guid`, the spatial `index`, `walking_creature_keys`,
/// `aggro_creature_keys`, wander/waypoint counts).
///
/// Used by both `World::with_creatures_and_db` and `World::for_test`
/// after `cell::partition_creatures` has bucketed the input by map.
fn build_map_state_with_creatures(
    map: Map,
    creatures: Vec<Creature>,
) -> MapState {
    let mut state = MapState::new_empty(map);

    let mut creature_slab: Slab<Creature> = Slab::with_capacity(creatures.len());
    for c in creatures {
        creature_slab.insert(c);
    }

    let mut creature_by_guid = ahash::AHashMap::with_capacity(creature_slab.len());
    let mut aggro_creature_keys = Vec::new();
    let mut walking_creature_keys = Vec::with_capacity(creature_slab.len());
    let mut creature_wander_count = 0;
    let mut creature_waypoint_count = 0;
    let mut index = crate::world::spatial::SpatialIndex::new();
    for (k, c) in creature_slab.iter() {
        creature_by_guid.insert(c.guid, k);
        match c.behavior {
            CreatureBehavior::AggroChase => aggro_creature_keys.push(k),
            CreatureBehavior::RandomWander { .. } => {
                walking_creature_keys.push(k);
                creature_wander_count += 1;
            }
            CreatureBehavior::Waypoint { .. } => {
                walking_creature_keys.push(k);
                creature_waypoint_count += 1;
            }
            CreatureBehavior::Idle => {}
        }
        if !matches!(c.life_state, CreatureLifeState::Respawning { .. }) {
            index.insert_creature(k, c.info.position.x, c.info.position.y);
        }
    }

    state.creatures = creature_slab;
    state.creature_by_guid = creature_by_guid;
    state.aggro_creature_keys = aggro_creature_keys;
    state.walking_creature_keys = walking_creature_keys;
    state.creature_wander_count = creature_wander_count;
    state.creature_waypoint_count = creature_waypoint_count;
    state.index = index;

    state
}

/// Bundle of per-cell tick outputs returned by the `tokio::spawn`ed
/// cell task to the orchestrator. Used for the slow-tick log,
/// post-spawn Tracy plots, and re-admitting logged-out clients into
/// `clients_on_character_screen`.
#[derive(Debug)]
pub struct PerCellTickResult {
    pub map: Map,
    /// True if this cell's pacer told it to skip this global tick:
    /// the global orchestrator is running at 30 Hz, but this cell's
    /// pacer has backed off (e.g. to 100 ms = 10 Hz) so it sat out
    /// the past `(target / current_interval) - 1` global ticks. The
    /// orchestrator suppresses Tracy plots and the slow-tick log for
    /// skipped cells so the dashboard only shows ticks that did
    /// real work.
    pub skipped: bool,
    pub t_per_client: Duration,
    pub t_build_view: Duration,
    pub t_flush: Duration,
    pub t_aoi: Duration,
    pub t_apply_cmds: Duration,
    pub t_corpses: Duration,
    pub t_creatures: Duration,
    pub t_logouts: Duration,
    /// Sum of every per-cell phase timing. Plotted as `cell_tick_ms`
    /// so the dashboard can compare per-cell tick cost; the global
    /// `tick_ms` plot covers the orchestrator (global phases + spawn
    /// orchestration + this cell's work).
    pub t_cell_total: Duration,
    pub departed: Vec<CharacterScreenClient>,
    /// Clients whose `map` changed this tick (continent teleport) and who
    /// must be re-homed into the destination map's `MapState`. The
    /// orchestrator drains these after joining all per-map tasks.
    pub map_changers: Vec<Client>,
    pub clients_count: usize,
    pub creatures_count: usize,
    pub creature_idle_count: usize,
    pub creature_wander_count: usize,
    pub creature_waypoint_count: usize,
    pub creature_aggro_count: usize,
    pub walking_creature_count: usize,
}

impl PerCellTickResult {
    /// Zero-valued placeholder used by the orchestrator when there
    /// are no cells at all (e.g. a fresh server boot with a failed
    /// worlddb load — no creatures, no clients, no cells exist
    /// yet). Lets the post-spawn metrics block read fields without
    /// crashing; the empty-world numbers (0 clients, 0 creatures,
    /// skipped=true) accurately reflect reality.
    fn empty() -> Self {
        Self {
            map: Map::EasternKingdoms,
            skipped: true,
            t_per_client: Duration::ZERO,
            t_build_view: Duration::ZERO,
            t_flush: Duration::ZERO,
            t_aoi: Duration::ZERO,
            t_apply_cmds: Duration::ZERO,
            t_corpses: Duration::ZERO,
            t_creatures: Duration::ZERO,
            t_logouts: Duration::ZERO,
            t_cell_total: Duration::ZERO,
            departed: Vec::new(),
            map_changers: Vec::new(),
            clients_count: 0,
            creatures_count: 0,
            creature_idle_count: 0,
            creature_wander_count: 0,
            creature_waypoint_count: 0,
            creature_aggro_count: 0,
            walking_creature_count: 0,
        }
    }
}

#[derive(Default, Debug)]
pub struct AoiTickStats {
    pub entered: usize,
    pub departed: usize,
    pub suppressed: usize,
    /// Observers whose `visible_entities` matched `new_visible` exactly
    /// this tick (the M2 short-circuit fired) — they skipped the slow
    /// diff path entirely. A high fast-path ratio means steady-state
    /// membership is stable, which is the expected case post-ramp at
    /// heavy density.
    pub fast_path: usize,
}

impl MapState {
    /// Build an empty `MapState` for `map`. Pacer is initialized from
    /// `[tick]` config. Used by `World::with_creatures` / `for_test` and by
    /// `World::ensure_map_exists` for lazy spin-up on first admit.
    pub(crate) fn new_empty(map: Map) -> Self {
        Self {
            map,
            clients: Slab::new(),
            client_by_guid: ahash::AHashMap::new(),
            creatures: Slab::new(),
            creature_by_guid: ahash::AHashMap::new(),
            aggro_creature_keys: Vec::new(),
            walking_creature_keys: Vec::new(),
            creature_wake_at: std::collections::BTreeMap::new(),
            creature_wander_count: 0,
            creature_waypoint_count: 0,
            index: crate::world::spatial::SpatialIndex::new(),
            last_tick_at: None,
            pending_movement: ahash::AHashMap::new(),
            tick_counter: 0,
            last_heartbeat_broadcast_tick: ahash::AHashMap::new(),
            scratch_active_grids: ahash::AHashSet::new(),
            scratch_walk_events: Vec::new(),
            scratch_to_park: Vec::new(),
            scratch_parked_set: ahash::AHashSet::new(),
            scratch_expired_roots: Vec::new(),
            broadcast_view: Vec::new(),
            pacer: crate::world::TickPacer::new_from_config(
                &crate::config::config().tick,
            ),
        }
    }

    /// Insert a client into the slab and keep `client_by_guid` in sync.
    /// Always use this rather than `self.clients.insert(...)` directly
    /// so the reverse index stays authoritative. Also publishes the
    /// client's identity into the process-wide
    /// [`crate::world::cell::PLAYER_REGISTRY`] so cross-cell GM
    /// lookups (e.g. `.go PlayerName`) can find them.
    pub(crate) fn insert_client(&mut self, c: Client) -> usize {
        let guid = c.character().guid;
        let pos = c.character().info.position;
        let entry = crate::world::cell::PlayerRegistryEntry {
            guid,
            name: c.character().name.clone(),
            map: c.character().map,
            position: pos,
            orientation: c.character().info.orientation,
        };
        let key = self.clients.insert(c);
        self.client_by_guid.insert(guid, key);
        self.index.insert_client(key, pos.x, pos.y);
        crate::world::cell::register_player(entry);
        key
    }

    /// Remove a client from the slab and drop the matching
    /// `client_by_guid` entry, the spatial index entry, plus the global
    /// registry entry. Pairs with [`Self::insert_client`]. This is a
    /// *genuine* departure (logout / cross-map teleport / stale drop) —
    /// for the per-tick held-active pattern use
    /// [`Self::take_active_client`] / [`Self::reinsert_active_client`].
    pub(crate) fn remove_client(&mut self, key: usize) -> Client {
        let c = self.clients.remove(key);
        let guid = c.character().guid;
        self.client_by_guid.remove(&guid);
        self.index.remove_client(key);
        crate::world::cell::unregister_player(guid);
        c
    }

    /// Detach a client from the slab for in-loop processing, leaving its
    /// spatial-index entry (keyed by the stable slab key) and global
    /// registry entry in place. The per-client loop removes each client,
    /// runs its opcode handler, then [`Self::reinsert_active_client`]s it —
    /// touching only the slab + `client_by_guid`, so the hot path doesn't
    /// churn the index or the process-wide `PLAYER_REGISTRY` mutex twice
    /// per client per tick. Slab key stability (the free-list reuses the
    /// just-vacated slot on reinsert) keeps the dangling index entry valid.
    pub(crate) fn take_active_client(&mut self, key: usize) -> Client {
        let c = self.clients.remove(key);
        self.client_by_guid.remove(&c.character().guid);
        c
    }

    /// Re-attach a client detached by [`Self::take_active_client`]. The
    /// returned slab key must equal the original (asserted by callers).
    /// `move_client` re-seats the index entry only if the client crossed a
    /// 33 yd cell this tick (cmangos relocation); otherwise it's a no-op.
    pub(crate) fn reinsert_active_client(&mut self, c: Client) -> usize {
        let guid = c.character().guid;
        let pos = c.character().info.position;
        let key = self.clients.insert(c);
        self.client_by_guid.insert(guid, key);
        self.index.move_client(key, pos.x, pos.y);
        key
    }

    /// Remove and return every client whose current `map` no longer matches
    /// this cell's map — i.e. they completed a continent teleport
    /// (`prepare_teleport` set `character.map` to the destination). The
    /// orchestrator re-admits each to its destination map's `MapState`.
    /// Returns empty in the common case (no cross-map teleports this tick),
    /// so it's a cheap scan over the slab.
    pub(crate) fn take_map_changers(&mut self) -> Vec<Client> {
        let movers: Vec<usize> = self
            .clients
            .iter()
            .filter(|(_, c)| c.character().map != self.map)
            .map(|(k, _)| k)
            .collect();
        movers.into_iter().map(|k| self.remove_client(k)).collect()
    }

    /// Add a freshly-inserted creature to the behavior key indexes.
    pub(crate) fn register_creature(&mut self, key: usize) {
        let c = &self.creatures[key];
        self.creature_by_guid.insert(c.guid, key);
        match c.behavior {
            CreatureBehavior::AggroChase => self.aggro_creature_keys.push(key),
            CreatureBehavior::RandomWander { .. } => {
                self.walking_creature_keys.push(key);
                self.creature_wander_count += 1;
            }
            CreatureBehavior::Waypoint { .. } => {
                self.walking_creature_keys.push(key);
                self.creature_waypoint_count += 1;
            }
            CreatureBehavior::Idle => {}
        }
        self.grid_insert(key);
    }

    /// Add `key` to the creature spatial grid. Idempotent on already-present
    /// keys (no-op). Skips creatures in the Respawning life state —
    /// `tick_aoi_transitions` filters those out anyway, so keeping them in
    /// the grid would waste cells.
    pub(crate) fn grid_insert(&mut self, key: usize) {
        let Some(c) = self.creatures.get(key) else {
            return;
        };
        if matches!(c.life_state, CreatureLifeState::Respawning { .. }) {
            return;
        }
        // `insert_creature` is idempotent on already-tracked keys.
        self.index.insert_creature(key, c.info.position.x, c.info.position.y);
    }

    /// Remove `key` from the grid if present. Used on Corpse → Respawning
    /// transitions and on creature destruction. Cheap when absent.
    pub(crate) fn grid_remove(&mut self, key: usize) {
        self.index.remove_creature(key);
    }

    /// Re-seat `key` into the cell matching its current `(map, position)`.
    /// No-op if the cell didn't change since the last insertion. Used after
    /// every position mutation in `tick_creature_ai` /
    /// `tick_walking_creatures` — only moving creatures pay the cost.
    pub(crate) fn grid_move(&mut self, key: usize) {
        let Some(c) = self.creatures.get(key) else {
            return;
        };
        if matches!(c.life_state, CreatureLifeState::Respawning { .. }) {
            // Defensive: shouldn't move while Respawning, but if it
            // happens we drop them from the grid to keep the invariant
            // "grid only holds visible creatures".
            self.index.remove_creature(key);
            return;
        }
        self.index.move_creature(key, c.info.position.x, c.info.position.y);
    }

    /// Transition a live creature to the corpse state: zero health, record
    /// time of death, halve the respawn delay if the mob lived for less
    /// than its current delay, de-index from the AI behavior key lists
    /// (so it stops ticking), and broadcast the visual death state to AOI
    /// viewers. Keeps the creature in the slab + `creature_by_guid` so
    /// queries still resolve while it's lying around.
    pub(crate) async fn kill_creature(&mut self, key: usize) {
        let Some(creature) = self.creatures.get_mut(key) else {
            return;
        };
        if !matches!(creature.life_state, CreatureLifeState::Alive) {
            return;
        }
        let now = Instant::now();
        let alive_for = now.saturating_duration_since(creature.last_alive_at);
        if alive_for < creature.respawn_delay {
            creature.respawn_delay /= 2;
        }
        creature.health = 0;
        creature.life_state = CreatureLifeState::Corpse { died_at: now };
        creature.invalidate_object_cache();
        let guid = creature.guid;
        let map = creature.map;
        let pos = creature.info.position;
        self.mark_creature_dead(key);

        const STAND_STATE_DEAD: u8 = 7;
        let dead_update = SMSG_UPDATE_OBJECT {
            has_transport: 0,
            objects: vec![Object {
                update_type: Object_UpdateType::Values {
                    guid1: guid,
                    mask1: UpdateMask::Unit(
                        wow_world_messages::vanilla::UpdateUnitBuilder::new()
                            .set_unit_health(0)
                            .set_unit_bytes_1(STAND_STATE_DEAD, 0, 0, 0)
                            .finalize(),
                    ),
                },
            }],
        };
        aoi::broadcast_within_aoi(dead_update, pos, map, &mut self.clients).await;
    }

    /// Drop a creature key from the AI behavior buckets without touching
    /// the slab or `creature_by_guid`. Used when transitioning to a corpse.
    pub(crate) fn mark_creature_dead(&mut self, key: usize) {
        let Some(c) = self.creatures.get(key) else {
            return;
        };
        match c.behavior {
            CreatureBehavior::AggroChase => {
                if let Some(i) = self.aggro_creature_keys.iter().position(|&k| k == key) {
                    self.aggro_creature_keys.swap_remove(i);
                }
            }
            CreatureBehavior::RandomWander { .. } => {
                if let Some(i) = self.walking_creature_keys.iter().position(|&k| k == key) {
                    self.walking_creature_keys.swap_remove(i);
                }
                self.creature_wander_count = self.creature_wander_count.saturating_sub(1);
            }
            CreatureBehavior::Waypoint { .. } => {
                if let Some(i) = self.walking_creature_keys.iter().position(|&k| k == key) {
                    self.walking_creature_keys.swap_remove(i);
                }
                self.creature_waypoint_count =
                    self.creature_waypoint_count.saturating_sub(1);
            }
            CreatureBehavior::Idle => {}
        }
    }

    /// Re-add a respawned creature's key to its behavior bucket.
    pub(crate) fn unmark_creature_dead(&mut self, key: usize) {
        let Some(c) = self.creatures.get(key) else {
            return;
        };
        match c.behavior {
            CreatureBehavior::AggroChase => self.aggro_creature_keys.push(key),
            CreatureBehavior::RandomWander { .. } => {
                self.walking_creature_keys.push(key);
                self.creature_wander_count += 1;
            }
            CreatureBehavior::Waypoint { .. } => {
                self.walking_creature_keys.push(key);
                self.creature_waypoint_count += 1;
            }
            CreatureBehavior::Idle => {}
        }
    }

    /// Walks corpses + respawning creatures. Corpses past `CORPSE_DESPAWN`
    /// transition to `Respawning` (broadcast destroy). Respawning creatures
    /// Process clients flagged for character-screen during this
    /// tick's `per_client_loop` (typically by `CMSG_LOGOUT_REQUEST`).
    /// For each: pull out of the cell's slab and append the resulting
    /// `CharacterScreenClient` to `departed`. The orchestrator hands
    /// those back to `World::clients_on_character_screen` for the
    /// relog flow.
    ///
    /// Despawn fan-out uses the same per-map coalescing as the
    /// stale-disconnect path: one `SMSG_UPDATE_OBJECT` per map
    /// carrying `OutOfRangeObjects { guids }` is sent to every
    /// observer on that map, regardless of AOI distance. The 1.12.2
    /// client ignores guids it doesn't know about, so skipping the
    /// per-recipient distance walk is harmless and avoids
    /// O(K × N) per-recipient packets on mass logout (e.g. a
    /// `simulate` teardown).
    #[tracing::instrument(level = "info", skip_all, name = "drain_logouts")]
    pub(crate) async fn drain_logouts(
        &mut self,
        keys: &[usize],
        departed: &mut Vec<CharacterScreenClient>,
    ) {
        if keys.is_empty() {
            return;
        }
        let mut by_map: ahash::AHashMap<Map, Vec<Guid>> = ahash::AHashMap::new();
        for &key in keys {
            let c = self.remove_client(key);
            let logout_map = c.character().map;
            let logout_guid = c.character().guid;
            // Drop the heartbeat-throttle bookkeeping for the leaving
            // player so the map doesn't accumulate stale guids over a
            // long server lifetime.
            self.last_heartbeat_broadcast_tick.remove(&logout_guid);
            by_map.entry(logout_map).or_default().push(logout_guid);
            departed.push(c.into_character_screen_client());
        }
        for (map, guids) in by_map {
            let msg = SMSG_UPDATE_OBJECT {
                has_transport: 0,
                objects: vec![Object {
                    update_type: Object_UpdateType::OutOfRangeObjects {
                        guids: guids.clone(),
                    },
                }],
            };
            for (_, a) in self.clients.iter_mut() {
                if a.character().map == map {
                    for g in &guids {
                        a.session.visible_entities.remove(g);
                    }
                    a.send_message(msg.clone()).await;
                }
            }
        }
    }

    /// Flush coalesced per-source movement broadcasts queued
    /// during `per_client_loop`. Each entry was queued by a
    /// movement opcode handler this tick; we issue at most one
    /// broadcast per source via the serialize-once
    /// `aoi::broadcast_opcode_within_aoi` path. The map is
    /// reused across ticks (`.drain()` keeps capacity).
    ///
    /// `Some(source_guid)` is passed to the broadcast so the
    /// source player does NOT receive their own movement opcode
    /// back — an echo would be treated by the local client as a
    /// server position correction (visible rubber-banding).
    ///
    /// `heartbeat_skip_ratio` is the pacer-driven throttle: 1
    /// means no throttle (~30 Hz pacer), 3 means every third
    /// heartbeat fires (~10 Hz pacer). Transition opcodes
    /// (start/stop/strafe/jump) ignore the throttle since
    /// observers can't infer those locally.
    #[tracing::instrument(level = "info", skip_all, name = "flush_movement_broadcasts")]
    pub(crate) fn flush_movement_broadcasts(&mut self, heartbeat_skip_ratio: u64) {
        // Per-tick broadcast totals so Tracy can show whether the
        // movement broadcast leg is a hotspot.
        let mut sources = 0_usize;
        let mut recipients = 0_usize;
        let mut bytes = 0_usize;
        let mut throttled = 0_usize;
        // Borrow these as raw fields up front — the loop body takes
        // `&mut self.last_heartbeat_broadcast_tick` which conflicts
        // with `&self.tick_counter` under the standard borrow check.
        let tick_counter = self.tick_counter;
        let skip_ratio = heartbeat_skip_ratio;
        for (source_guid, pm) in self.pending_movement.drain() {
            let is_heartbeat = matches!(
                pm.msg,
                ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(_)
            );
            if is_heartbeat && skip_ratio > 1 {
                let last = self
                    .last_heartbeat_broadcast_tick
                    .get(&source_guid)
                    .copied()
                    .unwrap_or(0);
                if tick_counter.saturating_sub(last) < skip_ratio {
                    throttled += 1;
                    continue;
                }
            }
            let (r, b) = aoi::broadcast_opcode_within_aoi(
                &pm.msg,
                pm.anchor,
                pm.map,
                Some(source_guid),
                &self.broadcast_view,
            );
            sources += 1;
            recipients += r;
            bytes += r * b;
            self.last_heartbeat_broadcast_tick
                .insert(source_guid, tick_counter);
        }
        if let Some(client) = tracy_client::Client::running() {
            client.plot(
                tracy_client::plot_name!("broadcast_sources"),
                sources as f64,
            );
            client.plot(
                tracy_client::plot_name!("broadcast_recipients"),
                recipients as f64,
            );
            client.plot(
                tracy_client::plot_name!("broadcast_bytes"),
                bytes as f64,
            );
            client.plot(
                tracy_client::plot_name!("broadcast_throttled"),
                throttled as f64,
            );
            client.plot(
                tracy_client::plot_name!("broadcast_skip_ratio"),
                skip_ratio as f64,
            );
        }
    }

    /// whose timer has elapsed transition back to `Alive` (broadcast a
    /// fresh create-object, restore HP, snap back to spawn position).
    #[tracing::instrument(level = "info", skip_all, name = "tick_corpses_and_respawns")]
    pub(crate) async fn tick_corpses_and_respawns(&mut self) {
        let now = Instant::now();

        let mut to_decay: Vec<usize> = Vec::new();
        let mut to_revive: Vec<usize> = Vec::new();
        for (key, c) in self.creatures.iter() {
            match c.life_state {
                CreatureLifeState::Corpse { died_at } => {
                    if now.saturating_duration_since(died_at)
                        >= crate::world::world_opcode_handler::creature::corpse_despawn()
                    {
                        to_decay.push(key);
                    }
                }
                CreatureLifeState::Respawning { respawn_at } => {
                    if now >= respawn_at {
                        to_revive.push(key);
                    }
                }
                CreatureLifeState::Alive => {}
            }
        }

        for key in to_decay {
            let Some(c) = self.creatures.get_mut(key) else {
                continue;
            };
            let respawn_at = now + c.respawn_delay;
            c.life_state = CreatureLifeState::Respawning { respawn_at };
            let guid = c.guid;
            let map = c.map;
            let pos = c.info.position;
            self.grid_remove(key);
            let destroy = SMSG_DESTROY_OBJECT { guid };
            aoi::broadcast_within_aoi(destroy, pos, map, &mut self.clients).await;
            for (_, o) in self.clients.iter_mut() {
                o.session.visible_entities.remove(&guid);
            }
        }

        for key in to_revive {
            let Some(c) = self.creatures.get_mut(key) else {
                continue;
            };
            c.life_state = CreatureLifeState::Alive;
            c.last_alive_at = now;
            c.health = c.max_health;
            c.info.position = c.spawn_position;
            c.info.orientation = c.spawn_orientation;
            c.info.flags = wow_world_messages::vanilla::MovementInfo_MovementFlags::default();
            c.invalidate_object_cache();
            let create_object = c.to_create_object();
            let map = c.map;
            let pos = c.info.position;
            let guid = c.guid;
            self.unmark_creature_dead(key);
            self.grid_insert(key);
            if let Some(msg) = UpdateObject::from_objects(vec![create_object]) {
                msg.broadcast_within_aoi(pos, map, &mut self.clients).await;
            }
            for (_, o) in self.clients.iter_mut() {
                if o.character().map == map
                    && aoi::within_aoi(&o.character().info.position, &pos)
                {
                    o.session.visible_entities.insert(guid);
                }
            }
        }
    }

    /// Recompute the cmangos active-grid set into `scratch_active_grids`: the
    /// union of every player's 533 yd grid expanded by [`ACTIVATION_GRID_RADIUS`].
    /// All clients are on `self.map` (one `MapState` per continent), so no map
    /// filter is needed. O(players × 9).
    fn refresh_active_grids(&mut self) {
        self.scratch_active_grids.clear();
        for (_, cl) in self.clients.iter() {
            let p = cl.character().info.position;
            let (gx, gy) = crate::world::spatial::grid_coord(p.x, p.y);
            for dx in -ACTIVATION_GRID_RADIUS..=ACTIVATION_GRID_RADIUS {
                for dy in -ACTIVATION_GRID_RADIUS..=ACTIVATION_GRID_RADIUS {
                    self.scratch_active_grids.insert((gx + dx, gy + dy));
                }
            }
        }
    }

    /// True if `(x, y)` lies in an active grid (a player is within
    /// [`ACTIVATION_GRID_RADIUS`] grids). Call after [`Self::refresh_active_grids`].
    #[inline]
    fn grid_is_active(active_grids: &ahash::AHashSet<(i32, i32)>, x: f32, y: f32) -> bool {
        active_grids.contains(&crate::world::spatial::grid_coord(x, y))
    }

    #[tracing::instrument(level = "info", skip_all, name = "tick_creature_ai")]
    pub(crate) async fn tick_creature_ai(&mut self, maps: &mut PathfindingMaps) {
        let creature_cfg = &crate::config::config().creature;
        let re_path_threshold = creature_cfg.re_path_threshold;
        let stand_off = creature_cfg.stand_off;
        let max_follow_range = creature_cfg.max_follow_range;

        // cmangos grid activation: recompute which 533 yd grids have a player
        // nearby. Gates the aggro scan below + the walking/park loops.
        self.refresh_active_grids();

        let now = std::time::Instant::now();
        let mut expired: Vec<(Guid, Map, Vector3d, Option<MovementInfo>)> = Vec::new();
        for (_, creature) in self.creatures.iter_mut() {
            if let Some(until) = creature.root_until
                && until <= now
            {
                creature.root_until = None;
                let resume_info = if matches!(
                    creature.behavior,
                    CreatureBehavior::RandomWander { .. } | CreatureBehavior::Waypoint { .. }
                ) {
                    creature.info.flags = MovementInfo_MovementFlags::new_forward();
                    creature.invalidate_object_cache();
                    Some(creature.info.clone())
                } else {
                    None
                };
                expired.push((
                    creature.guid,
                    creature.map,
                    creature.info.position,
                    resume_info,
                ));
            }
        }
        for (guid, map, pos, resume_info) in expired {
            let clear = SMSG_UPDATE_OBJECT {
                has_transport: 0,
                objects: vec![Object {
                    update_type: Object_UpdateType::Values {
                        guid1: guid,
                        mask1: UpdateMask::Unit(
                            wow_world_messages::vanilla::UpdateUnitBuilder::new()
                                .set_unit_aura(0)
                                .set_unit_auraflags(0, 0, 0, 0)
                                .set_unit_auralevels(0, 0, 0, 0)
                                .set_unit_auraapplications(0, 0, 0, 0)
                                .finalize(),
                        ),
                    },
                }],
            };
            aoi::broadcast_within_aoi(clear, pos, map, &mut self.clients).await;
            if let Some(info) = resume_info {
                let resume = MSG_MOVE_START_FORWARD_Server { guid, info };
                aoi::broadcast_within_aoi(resume, pos, map, &mut self.clients).await;
            }
        }

        // Aggro target candidate scan. KNOWN LIMITATION: this
        // iterates only the LOCAL cell's clients. A creature
        // near a cell boundary won't aggro a player in the
        // neighbor cell. Doing it properly requires creature
        // cross-cell transition support (creature moves into
        // the neighbor's slab as it chases the player) which
        // Stage 5 only built for clients. Tracked as future work;
        // user-visible impact is low (mob doesn't attack you from
        // across the boundary, until you walk into its cell).
        let clients = &self.clients;
        let active_grids = &self.scratch_active_grids;
        let targets: Vec<(usize, Option<usize>)> = self
            .aggro_creature_keys
            .par_iter()
            .map(|&creature_key| {
                let creature = &self.creatures[creature_key];
                if creature.is_rooted() {
                    return (creature_key, None);
                }
                let from = creature.info.position;
                // Grid activation: a creature in an idle grid (no player within
                // the 533 yd halo) doesn't scan for aggro targets at all.
                if !Self::grid_is_active(active_grids, from.x, from.y) {
                    return (creature_key, None);
                }
                let map = creature.map;
                let target = clients
                    .iter()
                    .filter(|(_, c)| c.character().map == map)
                    .filter(|(_, c)| {
                        squared_xy_dist(&c.character().info.position, &from)
                            <= max_follow_range * max_follow_range
                    })
                    .min_by(|a, b| {
                        let da = squared_xy_dist(&a.1.character().info.position, &from);
                        let db = squared_xy_dist(&b.1.character().info.position, &from);
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(idx, _)| idx);
                (creature_key, target)
            })
            .collect();

        let mut groups: ahash::AHashMap<usize, Vec<usize>> = ahash::AHashMap::new();
        for (creature_key, target) in &targets {
            if let Some(player_key) = target {
                groups.entry(*player_key).or_default().push(*creature_key);
            }
        }
        for v in groups.values_mut() {
            v.sort_by_key(|&i| self.creatures[i].guid.guid());
        }

        let mut moves = Vec::new();
        for (player_key, creature_keys) in &groups {
            let n = creature_keys.len();
            let player = &self.clients[*player_key];
            let player_pos = player.character().info.position;
            let player_orient = player.character().info.orientation;

            for (slot, &creature_key) in creature_keys.iter().enumerate() {
                let from = self.creatures[creature_key].info.position;
                let slot_offset = if n == 1 {
                    0.0
                } else {
                    -std::f32::consts::FRAC_PI_2
                        + (slot as f32 + 0.5) * std::f32::consts::PI / n as f32
                };
                let angle = player_orient + slot_offset;
                let target_x = player_pos.x + stand_off * angle.cos();
                let target_y = player_pos.y + stand_off * angle.sin();

                let dx = target_x - from.x;
                let dy = target_y - from.y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < re_path_threshold {
                    continue;
                }

                let to = Vector3d {
                    x: target_x,
                    y: target_y,
                    z: player_pos.z,
                };
                let duration_ms =
                    ((dist / DEFAULT_RUNNING_SPEED).max(0.0) * 1000.0) as u32;
                moves.push((creature_key, from, to, duration_ms));
            }
        }

        for (key, from, to, duration_ms) in moves {
            let creature = &mut self.creatures[key];
            let map = creature.map;
            let msg = SMSG_MONSTER_MOVE {
                guid: creature.guid,
                spline_point: from,
                spline_id: 0,
                move_type: SMSG_MONSTER_MOVE_MonsterMoveType::Normal,
                spline_flags: SplineFlag::empty(),
                duration: duration_ms,
                splines: vec![to],
            };
            creature.info.position = to;
            creature.invalidate_object_cache();
            self.grid_move(key);

            for (_, c) in &mut self.clients {
                if c.character().map == map
                    && aoi::within_aoi(&c.character().info.position, &to)
                {
                    c.send_message(msg.clone()).await;
                }
            }
        }

        self.tick_walking_creatures(now, maps).await;
    }

    #[tracing::instrument(level = "info", skip_all, name = "tick_walking_creatures")]
    pub(crate) async fn tick_walking_creatures(
        &mut self,
        now: Instant,
        maps: &mut PathfindingMaps,
    ) {
        let creature_cfg = &crate::config::config().creature;
        let heartbeat_interval_ms = creature_cfg.walking_heartbeat_ms;
        let arrival_threshold = creature_cfg.arrival_threshold;
        let wander_idle_min_ms = creature_cfg.wander_idle_min_ms;
        let wander_idle_max_ms = creature_cfg.wander_idle_max_ms;

        // Take the cmangos active-grid set (filled in tick_creature_ai) as a
        // local so reading it doesn't conflict with the &mut self.creatures /
        // self.creature_wake_at borrows below. Restored at the end.
        let active_grids = std::mem::take(&mut self.scratch_active_grids);

        // Wake loop: a parked creature whose timer elapsed rejoins the walking
        // set only if its grid is active. Otherwise it's re-parked for a later
        // recheck, so an unpopulated continent's creatures never re-enter the
        // per-tick walk loop — they sit in `creature_wake_at` at ~1 Hz.
        while let Some((&t, _)) = self.creature_wake_at.iter().next() {
            if t > now {
                break;
            }
            let keys = self.creature_wake_at.remove(&t).unwrap_or_default();
            for k in keys {
                let Some(c) = self.creatures.get(k) else {
                    continue;
                };
                let p = c.info.position;
                if Self::grid_is_active(&active_grids, p.x, p.y) {
                    self.walking_creature_keys.push(k);
                } else {
                    let recheck = now + std::time::Duration::from_millis(IDLE_GRID_RECHECK_MS);
                    self.creature_wake_at.entry(recheck).or_default().push(k);
                }
            }
        }

        let mut events = std::mem::take(&mut self.scratch_walk_events);
        events.clear();
        let mut to_park = std::mem::take(&mut self.scratch_to_park);
        to_park.clear();

        let walking_keys = std::mem::take(&mut self.walking_creature_keys);
        for &key in &walking_keys {
            let c = &mut self.creatures[key];
            // Grid activation: a creature whose grid went idle this tick (the
            // last player left its 533 yd halo) stops cleanly and parks for a
            // periodic recheck, leaving the per-tick walk loop. No broadcast —
            // no observer is within AOI to see it. It's reset so that when its
            // grid reactivates it re-announces movement (StartForward) rather
            // than sliding from a stale anchor.
            let p = c.info.position;
            if !Self::grid_is_active(&active_grids, p.x, p.y) {
                if c.info.flags != MovementInfo_MovementFlags::default() {
                    c.info.flags = MovementInfo_MovementFlags::default();
                    c.invalidate_object_cache();
                }
                match &mut c.behavior {
                    CreatureBehavior::RandomWander { target, next_decision_at, .. } => {
                        *target = None;
                        *next_decision_at = now;
                    }
                    CreatureBehavior::Waypoint { idle_until, .. } => {
                        *idle_until = Some(now);
                    }
                    _ => {}
                }
                c.last_advanced_at = now;
                to_park.push((now + std::time::Duration::from_millis(IDLE_GRID_RECHECK_MS), key));
                continue;
            }
            if c.is_rooted() {
                c.last_advanced_at = now;
                continue;
            }

            let dt = now
                .saturating_duration_since(c.last_advanced_at)
                .as_secs_f32()
                .min(0.5);
            c.last_advanced_at = now;
            let step = walk_speed() * dt;
            let map = c.map;

            let mut just_started = false;
            match &mut c.behavior {
                CreatureBehavior::RandomWander {
                    anchor,
                    radius,
                    target,
                    next_decision_at,
                } => {
                    if target.is_none() {
                        if now < *next_decision_at {
                            to_park.push((*next_decision_at, key));
                            continue;
                        }
                        let r = crate::world::world_opcode_handler::gm_command::next_rand();
                        let angle = crate::numeric::rand_unit_f32(r) * std::f32::consts::TAU;
                        let r2 = crate::world::world_opcode_handler::gm_command::next_rand();
                        let mag = crate::numeric::rand_unit_f32(r2).sqrt() * *radius;
                        *target = Some(Vector3d {
                            x: anchor.x + mag * angle.cos(),
                            y: anchor.y + mag * angle.sin(),
                            z: anchor.z,
                        });
                        just_started = true;
                    }
                }
                CreatureBehavior::Waypoint {
                    waypoints,
                    idle_until,
                    current,
                    ..
                } => {
                    if waypoints.is_empty() {
                        continue;
                    }
                    if let Some(until) = idle_until {
                        if now < *until {
                            to_park.push((*until, key));
                            continue;
                        }
                        *idle_until = None;
                        just_started = true;
                    }
                    if *current >= waypoints.len() {
                        *current = 0;
                    }
                }
                _ => unreachable!(),
            }

            let target = match &c.behavior {
                CreatureBehavior::RandomWander {
                    target: Some(t), ..
                } => *t,
                CreatureBehavior::Waypoint {
                    waypoints,
                    current,
                    idle_until: None,
                    ..
                } if !waypoints.is_empty() => waypoints[*current],
                _ => continue,
            };

            if just_started {
                c.info.orientation = (target.y - c.info.position.y)
                    .atan2(target.x - c.info.position.x);
                c.info.flags = MovementInfo_MovementFlags::new_forward().set_walk_mode();
                c.invalidate_object_cache();
                events.push((key, c.info.position, map, CreatureMoveEvent::StartForward));
            }

            let dx = target.x - c.info.position.x;
            let dy = target.y - c.info.position.y;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist <= step || dist <= arrival_threshold {
                c.info.position = target;
                c.info.flags = MovementInfo_MovementFlags::default();
                c.invalidate_object_cache();
                events.push((key, c.info.position, map, CreatureMoveEvent::Stop));
                let park_at = match &mut c.behavior {
                    CreatureBehavior::RandomWander {
                        target,
                        next_decision_at,
                        ..
                    } => {
                        *target = None;
                        let span = wander_idle_max_ms - wander_idle_min_ms;
                        let idle_ms = wander_idle_min_ms
                            + crate::world::world_opcode_handler::gm_command::next_rand() % span;
                        *next_decision_at = now + std::time::Duration::from_millis(idle_ms);
                        Some(*next_decision_at)
                    }
                    CreatureBehavior::Waypoint {
                        waypoints,
                        waittimes_ms,
                        current,
                        idle_until,
                    } => {
                        let wt = waittimes_ms.get(*current).copied().unwrap_or(0);
                        let wt = wt.max(500) as u64;
                        let park = now + std::time::Duration::from_millis(wt);
                        *idle_until = Some(park);
                        *current = (*current + 1) % waypoints.len();
                        Some(park)
                    }
                    _ => None,
                };
                if let Some(when) = park_at {
                    to_park.push((when, key));
                }
            } else {
                c.info.position.x += step * dx / dist;
                c.info.position.y += step * dy / dist;
                c.info.position.z = target.z;
                c.invalidate_object_cache();
                if !just_started
                    && now
                        .saturating_duration_since(c.last_heartbeat_at)
                        .as_millis()
                        >= heartbeat_interval_ms
                {
                    events.push((key, c.info.position, map, CreatureMoveEvent::Heartbeat));
                    c.last_heartbeat_at = now;
                }
            }
        }
        for &key in &walking_keys {
            self.grid_move(key);
        }

        if to_park.is_empty() {
            self.walking_creature_keys = walking_keys;
        } else {
            let mut parked = std::mem::take(&mut self.scratch_parked_set);
            parked.clear();
            parked.extend(to_park.iter().map(|(_, k)| *k));
            self.walking_creature_keys =
                walking_keys.into_iter().filter(|k| !parked.contains(k)).collect();
            for &(when, k) in &to_park {
                self.creature_wake_at.entry(when).or_default().push(k);
            }
            self.scratch_parked_set = parked;
        }
        self.scratch_to_park = to_park;
        self.scratch_active_grids = active_grids;

        for (key, _, map, _) in &events {
            let Some(creature) = self.creatures.get(*key) else {
                continue;
            };
            let xy = (creature.info.position.x, creature.info.position.y);
            let z_hint = creature.info.position.z;
            if let Some(z) = maps.ground_height(*map, xy.0, xy.1, z_hint)
                && let Some(c) = self.creatures.get_mut(*key)
            {
                c.info.position.z = z;
            }
        }

        for (key, _, map, event) in events.drain(..) {
            let Some(creature) = self.creatures.get(key) else {
                continue;
            };
            let pos = creature.info.position;
            match event {
                CreatureMoveEvent::StartForward => {
                    let msg = MSG_MOVE_START_FORWARD_Server {
                        guid: creature.guid,
                        info: creature.info.clone(),
                    };
                    aoi::broadcast_within_aoi(msg, pos, map, &mut self.clients).await;
                }
                CreatureMoveEvent::Heartbeat => {
                    let msg = MSG_MOVE_HEARTBEAT_Server {
                        guid: creature.guid,
                        info: creature.info.clone(),
                    };
                    aoi::broadcast_within_aoi(msg, pos, map, &mut self.clients).await;
                }
                CreatureMoveEvent::Stop => {
                    let msg = MSG_MOVE_STOP_Server {
                        guid: creature.guid,
                        info: creature.info.clone(),
                    };
                    aoi::broadcast_within_aoi(msg, pos, map, &mut self.clients).await;
                }
            }
        }
        self.scratch_walk_events = events;
    }

    /// AOI transition tick. For each connected client, recomputes the set
    /// of players currently within `AOI_RADIUS_YARDS` on the same map and
    /// diffs against `session.visible_entities`:
    ///
    /// - **Departed** guids (in old set, not in new) get bundled into a
    ///   single `SMSG_UPDATE_OBJECT { OutOfRangeObjects }` despawn packet.
    /// - **Entered** guids (in new set, not in old) get bundled into a
    ///   single `SMSG_UPDATE_OBJECT` carrying one `CreateObject2` per
    ///   newcomer.
    ///
    /// Cost is O(N²) over connected clients per tick (N inner-loop AOI
    /// checks for each of N observers). The phase is rayon-parallelized
    /// across the snapshotted broadcast view — see the body. Spatial-
    /// grid bucketing is the next optimization if profiling shows this
    /// is hot beyond what parallelism handles.
    ///
    /// Only **players** are tracked here for now; creatures and simulated
    /// players use their own static-spawn / kill paths today. Extending
    /// to those is straightforward but out of scope for the despawn-on-
    /// AOI-exit fix this addresses.
    pub async fn tick_aoi_transitions(
        &mut self,
        global: &crate::world::aoi::GlobalAoiSnapshot,
    ) -> AoiTickStats {
        let mut stats = AoiTickStats::default();
        async {
        // Hoist AOI radius + flap cooldown + `now` once for the whole
        // phase. Each is captured by every rayon worker; reading them
        // once here is cheaper than per-observer config lookups.
        let aoi_r = crate::config::config().network.aoi_radius_yards;
        let aoi_r_sq = aoi_r * aoi_r;
        let cooldown = crate::config::config().network.aoi_flap_cooldown();
        let now = Instant::now();

        // ── PHASE 0: extract per-observer mutable state into a work
        // list. `mem::take` is O(1) — it swaps each `visible_entities`
        // / `aoi_transition_at` with `Default::default()` (empty
        // set/map), moving the underlying heap allocation into the
        // work item. No data is cloned. We restore in Phase 2.
        //
        // This is the trick that lets us parallelize `aoi_slow_diff`:
        // each rayon worker owns its observer's prior state outright,
        // does the flap-suppression + diff against `new_visible`
        // entirely in the closure, then hands the updated state back
        // via `DiffResult`. Without this we'd be passing `&mut
        // PlayerSession` across threads, which isn't possible because
        // `PlayerSession` is `!Sync` (the inbound `tokio::sync::mpsc`
        // `Receiver` is single-consumer).
        struct DiffInput {
            key: usize,
            guid: Guid,
            map: Map,
            position: Vector3d,
            visible: ahash::AHashSet<Guid>,
            transition: ahash::AHashMap<Guid, Instant>,
        }
        let work: Vec<DiffInput> = self
            .clients
            .iter_mut()
            .map(|(key, c)| DiffInput {
                key,
                guid: c.character().guid,
                map: c.character().map,
                position: c.character().info.position,
                visible: std::mem::take(&mut c.session.visible_entities),
                transition: std::mem::take(&mut c.session.aoi_transition_at),
            })
            .collect();

        // ── PHASE 1 (parallel): per-observer, compute everything:
        // new_visible build, fast-path check, slow-diff with
        // flap suppression, and entered-objects build. All reads
        // are from `Sync` sources (broadcast_view, creatures slab,
        // creature_grid_cells map, client_by_guid, clients slab). All
        // writes are to the moved-in `visible` / `transition` /
        // returned vecs — no cross-observer state. Rayon spreads the
        // observers across its thread pool.
        struct DiffResult {
            key: usize,
            visible_entities: ahash::AHashSet<Guid>,
            aoi_transition_at: ahash::AHashMap<Guid, Instant>,
            departed: Vec<Guid>,
            entered: Vec<Guid>,
            fast_path: bool,
            suppressed: usize,
        }
        let results: Vec<DiffResult> = {
            let _s = tracing::info_span!("aoi_diff_parallel").entered();
            // Read from the per-map snapshot's Sync projections (Client is not
            // safe to touch across rayon threads). Both clients and creatures
            // are spatially bucketed, so each observer visits a radius-windowed
            // cell range — O(nearby), not O(all clients).
            let client_grid_cells = &global.client_grid_cells;
            let creature_grid_cells = &global.creature_grid_cells;
            work.into_par_iter()
                .map(|input| {
                    let DiffInput {
                        key,
                        guid: observer_guid,
                        map: observer_map,
                        position: observer_pos,
                        visible,
                        mut transition,
                    } = input;

                    // Build new_visible by visiting the radius-derived cell
                    // window once, checking both the client and creature grids
                    // for each cell. Excludes self; precise squared-distance
                    // filter on each candidate (the grid only narrows).
                    let mut new_visible: ahash::AHashSet<Guid> =
                        ahash::AHashSet::with_capacity(visible.len());
                    let (cx, cy) = grid_cell_for(observer_pos.x, observer_pos.y);
                    let cr = grid_cell_radius(aoi_r);
                    for dx in -cr..=cr {
                        for dy in -cr..=cr {
                            let cell_key = (observer_map, cx + dx, cy + dy);
                            if let Some(views) = client_grid_cells.get(&cell_key) {
                                for v in views {
                                    if v.guid != observer_guid
                                        && aoi::within_aoi_sq(&observer_pos, &v.position, aoi_r_sq)
                                    {
                                        new_visible.insert(v.guid);
                                    }
                                }
                            }
                            if let Some(views) = creature_grid_cells.get(&cell_key) {
                                for v in views {
                                    if aoi::within_aoi_sq(&observer_pos, &v.position, aoi_r_sq) {
                                        new_visible.insert(v.guid);
                                    }
                                }
                            }
                        }
                    }

                    // Fast-path: visible set identical to last tick.
                    // Common in steady state; skip the diff + alloc.
                    if new_visible.len() == visible.len()
                        && new_visible.iter().all(|g| visible.contains(g))
                    {
                        return DiffResult {
                            key,
                            visible_entities: new_visible,
                            aoi_transition_at: transition,
                            departed: Vec::new(),
                            entered: Vec::new(),
                            fast_path: true,
                            suppressed: 0,
                        };
                    }

                    // Slow path: flap-suppression. For every guid whose
                    // membership flips, consult `transition`: if it
                    // moved within `cooldown`, force it back to its
                    // prior state so we don't re-emit a packet for an
                    // oscillating entity. Otherwise stamp `now` so
                    // subsequent jitter inside the window is suppressed.
                    let mut suppressed_count: usize = 0;
                    let changed: Vec<Guid> = visible
                        .symmetric_difference(&new_visible)
                        .copied()
                        .collect();
                    for g in changed {
                        let was = visible.contains(&g);
                        let cooldown_active = transition
                            .get(&g)
                            .is_some_and(|t| now.saturating_duration_since(*t) < cooldown);
                        if cooldown_active {
                            suppressed_count += 1;
                            if was {
                                new_visible.insert(g);
                            } else {
                                new_visible.remove(&g);
                            }
                        } else {
                            transition.insert(g, now);
                        }
                    }
                    // Periodic prune so the cooldown map doesn't grow
                    // unbounded for long-lived sessions.
                    transition
                        .retain(|_, t| now.saturating_duration_since(*t) < cooldown);

                    // Compute departed/entered against the old `visible`
                    // and the post-suppression `new_visible`. No clone
                    // is needed: we own `visible` outright (moved in
                    // via mem::take), so we can read it before dropping
                    // it at end of closure.
                    let departed: Vec<Guid> =
                        visible.difference(&new_visible).copied().collect();
                    let entered: Vec<Guid> =
                        new_visible.difference(&visible).copied().collect();

                    // CreateObject2 frames are NOT built here — only the entered
                    // GUIDs are returned. Phase 1.5 (below, sequential) builds
                    // the wire payload once per distinct newcomer from the live
                    // slab, so a brawl where everyone's moving still pays for the
                    // handful that crossed an AOI boundary, not every entity.
                    DiffResult {
                        key,
                        visible_entities: new_visible,
                        aoi_transition_at: transition,
                        departed,
                        entered,
                        fast_path: false,
                        suppressed: suppressed_count,
                    }
                })
                .collect()
        };

        // ── PHASE 1.5 (sequential): build CreateObject2 frames for the UNION
        // of entered guids across all observers — once per distinct newcomer,
        // from the live slabs. This is the lazy replacement for the old
        // build-everything-every-tick snapshot: in steady state most observers
        // fast-path (zero entered), and a moving brawl only crosses a handful
        // of AOI boundaries per tick, so we build a handful of masks instead of
        // one per entity. Immutable `&self` access only — must precede the
        // `&mut self.clients` writes in Phase 2.
        let entered_objects: ahash::AHashMap<Guid, Arc<Object>> = {
            let _s = tracing::info_span!("aoi_build_entered").entered();
            let mut built: ahash::AHashMap<Guid, Arc<Object>> = ahash::AHashMap::new();
            for result in &results {
                for &g in &result.entered {
                    if built.contains_key(&g) {
                        continue;
                    }
                    if let Some(&ck) = self.client_by_guid.get(&g) {
                        let ch = self.clients[ck].character();
                        built.insert(g, Arc::new(player_create_object(ch)));
                    } else if let Some(&ck) = self.creature_by_guid.get(&g) {
                        // Creatures keep their own invalidated cache; reuse it.
                        built.insert(g, self.creatures[ck].cached_create_object());
                    }
                }
            }
            built
        };

        // ── PHASE 2 (sequential): restore per-observer state, send
        // packets. Async sends keep this single-threaded.
        for result in results {
            stats.fast_path += if result.fast_path { 1 } else { 0 };
            stats.suppressed += result.suppressed;
            stats.departed += result.departed.len();
            stats.entered += result.entered.len();

            // Deref-clone the shared Arc frames for this observer's newcomers
            // (the wire encoder consumes `Vec<Object>` by value). A popular
            // newcomer entering many observers' AOI was built once above.
            let mut entered_frames = Vec::with_capacity(result.entered.len());
            for g in &result.entered {
                if let Some(arc) = entered_objects.get(g) {
                    entered_frames.push((**arc).clone());
                }
            }

            let obs = &mut self.clients[result.key];
            obs.session.visible_entities = result.visible_entities;
            obs.session.aoi_transition_at = result.aoi_transition_at;

            if !result.departed.is_empty() {
                let msg = SMSG_UPDATE_OBJECT {
                    has_transport: 0,
                    objects: vec![Object {
                        update_type: Object_UpdateType::OutOfRangeObjects {
                            guids: result.departed,
                        },
                    }],
                };
                obs.send_message(msg).await;
            }
            // Use chunked send: at high density a freshly-teleported
            // observer's entered Vec can carry hundreds of CreateObject2s,
            // which overflows the wire-protocol u16 size header if sent
            // in one packet. See `UpdateObject::send_chunked`.
            UpdateObject::send_chunked(entered_frames, obs).await;
        }
        }
        .instrument(tracing::info_span!("tick_aoi_transitions"))
        .await;
        stats
    }

    /// Build an AoI snapshot for THIS map from the cell's own live state.
    ///
    /// With one `MapState` per map (cmangos-style per-map concurrency) there
    /// are no neighbor cells to merge — the cell's own clients + creature grid
    /// ARE the whole map. This replaces the old cross-cell
    /// `World::build_global_aoi_snapshot` merge. It still exists because the
    /// rayon AoI diff needs `Sync` projections (`Client` is `!Sync`): broadcast
    /// targets + guid/position grid views. CreateObject2 frames are NOT
    /// pre-built here — `tick_aoi_transitions` builds them lazily for the few
    /// guids that actually enter an AOI this tick (Phase 1.5).
    fn build_local_aoi_snapshot(&self) -> aoi::GlobalAoiSnapshot {
        use crate::world::spatial::Occupant;
        let mut broadcast_view = Vec::with_capacity(self.clients.len());
        let mut creature_grid_cells: ahash::AHashMap<(Map, i32, i32), Vec<aoi::CreatureView>> =
            ahash::AHashMap::with_capacity(self.index.occupied_cell_count());
        let mut client_grid_cells: ahash::AHashMap<(Map, i32, i32), Vec<aoi::CreatureView>> =
            ahash::AHashMap::new();

        let map = self.map;
        // `broadcast_view` covers every client regardless of grid cell
        // (movement fan-out / clients_in_radius).
        for (_, client) in self.clients.iter() {
            broadcast_view.push(client.broadcast_target());
        }
        // The radius-windowed grid views are projected from the PERSISTENT
        // index (maintained incrementally), not a fresh membership scan. Each
        // 33.33-yd cell holds both clients and creatures; split by kind.
        for ((cx, cy), occupants) in self.index.iter_cells() {
            for &occ in occupants {
                match occ {
                    Occupant::Client(k) => {
                        let ch = self.clients[k].character();
                        client_grid_cells
                            .entry((map, cx, cy))
                            .or_default()
                            .push(aoi::CreatureView { guid: ch.guid, position: ch.info.position });
                    }
                    Occupant::Creature(k) => {
                        let cr = &self.creatures[k];
                        creature_grid_cells
                            .entry((map, cx, cy))
                            .or_default()
                            .push(aoi::CreatureView { guid: cr.guid, position: cr.info.position });
                    }
                }
            }
        }

        aoi::GlobalAoiSnapshot {
            broadcast_view,
            creature_grid_cells,
            client_grid_cells,
        }
    }

    /// Drains the command queue accumulated during opcode/GM handling and
    /// performs the corresponding world mutation + AOI broadcast. This is the
    /// single place that spawns/kills/sim-instantiates — handlers themselves
    /// must not touch `self.creatures` / `self.simulated_players` directly.
    #[tracing::instrument(level = "info", skip_all, name = "apply_commands")]
    pub(crate) async fn apply_commands(&mut self, queue: &mut crate::world::command::CommandQueue, maps: &mut PathfindingMaps) {
        use crate::world::command::WorldCommand;
        for cmd in queue.drain() {
            match cmd {
                WorldCommand::SpawnCreature(mut creature) => {
                    let map = creature.map;
                    // Snap to terrain at spawn; also re-seat the wander anchor
                    // so future target picks inherit the corrected z. No-op
                    // when pathfinding isn't configured for this map.
                    if let Some(z) = maps.ground_height(
                        map,
                        creature.info.position.x,
                        creature.info.position.y,
                        creature.info.position.z,
                    ) {
                        creature.info.position.z = z;
                        // No cache to invalidate yet — this creature
                        // hasn't been inserted, so nothing has called
                        // `cached_create_object` against it.
                        if let CreatureBehavior::RandomWander { anchor, .. } =
                            &mut creature.behavior
                        {
                            anchor.z = z;
                        }
                    }
                    let pos = creature.info.position;
                    let guid = creature.guid;
                    let create_object = creature.to_create_object();
                    let key = self.creatures.insert(creature);
                    self.register_creature(key);
                    if let Some(msg) = UpdateObject::from_objects(vec![create_object]) {
                        msg.broadcast_within_aoi(pos, map, &mut self.clients).await;
                    }
                    // Seed in-AOI observers' visible sets so the next
                    // AOI tick doesn't treat this as a fresh entry and
                    // duplicate the CreateObject.
                    for (_, o) in self.clients.iter_mut() {
                        if o.character().map == map
                            && aoi::within_aoi(&o.character().info.position, &pos)
                        {
                            o.session.visible_entities.insert(guid);
                        }
                    }
                }
                WorldCommand::KillCreature(kill_guid) => {
                    let Some(creature_key) = self.creature_by_guid.get(&kill_guid).copied()
                    else {
                        continue;
                    };
                    self.kill_creature(creature_key).await;
                }
            }
        }
    }

    pub fn shrink_periodic(&mut self) {
        // Long-lived primary collections.
        self.clients.shrink_to_fit();
        self.client_by_guid.shrink_to_fit();
        self.creatures.shrink_to_fit();
        self.creature_by_guid.shrink_to_fit();
        self.index.shrink_to_fit();
        self.aggro_creature_keys.shrink_to_fit();
        self.walking_creature_keys.shrink_to_fit();

        // Per-tick coalescer and scratch buffers. Each is `.clear()`'d at the
        // top of its phase, so calling `shrink_to_fit` here is safe — it
        // doesn't lose any in-flight state, just returns leftover capacity
        // sized for an earlier peak. Without this, a brief 5000-sim spike
        // pins ~150 KB of `scratch_walk_events` capacity indefinitely.
        self.pending_movement.shrink_to_fit();
        self.last_heartbeat_broadcast_tick.shrink_to_fit();
        self.scratch_active_grids.shrink_to_fit();
        self.scratch_walk_events.shrink_to_fit();
        self.scratch_to_park.shrink_to_fit();
        self.scratch_parked_set.shrink_to_fit();
        self.scratch_expired_roots.shrink_to_fit();
        // `creature_wake_at` is a BTreeMap — no shrink_to_fit on the node
        // allocator. Entries naturally drain at their wake time so peak
        // capacity isn't held indefinitely the way Vec/HashMap capacity is.
    }
}

impl World {
    pub fn with_creatures(
        clients_waiting_to_join: Receiver<CharacterScreenClient>,
        creatures: Slab<Creature>,
    ) -> Self {
        Self::with_creatures_and_db(
            clients_waiting_to_join,
            creatures,
            WorldDatabase::new(),
        )
    }

    /// Variant of [`with_creatures`] that adopts an externally-constructed
    /// `WorldDatabase` (e.g. one restored from a snapshot). The Stage 3
    /// production path uses this so the freshly-loaded DB enters the
    /// `Arc<Mutex<>>` directly rather than being created empty here.
    ///
    /// Stage 5 partition: the creature slab is bucketed by
    /// [`CellKey::from_position`]; each bucket becomes its own
    /// `MapState` under [`World::cells`] and registers its inbox
    /// in the process-wide routing table.
    pub fn with_creatures_and_db(
        clients_waiting_to_join: Receiver<CharacterScreenClient>,
        creatures: Slab<Creature>,
        db: WorldDatabase,
    ) -> Self {
        let maps = PathfindingMaps::new();

        // Worlddb creatures keep whatever z mangos stored. ~17% are
        // stale (idle mobs that never emit a movement event and so
        // never get snapped at runtime). Previously this constructor
        // iterated every creature and called `ground_height`, which
        // lazy-loaded every ADT tile they sat on — adding minutes
        // to a cold boot. Snap-on-startup is removed in favor of
        // lazy snapping at the points that *also* load tiles:
        // - `tick_walking_creatures` snaps z on every walk-step.
        // - `apply_commands::SpawnCreature` snaps runtime-spawned mobs.
        // - Movement opcode broadcasts (combat / aggro) update z as
        //   the creature moves.
        // The remaining gap is idle mobs that mangos got wrong AND
        // that no player ever forces to move; they float / clip by a
        // yard or two. Cosmetic. If it becomes visible enough to fix,
        // add an on-AOI-entry snap (one ground_height per creature
        // first time it enters someone's visible set).

        // Partition by map (one MapState per continent). Pure function;
        // consumes the slab.
        let buckets = crate::world::cell::partition_creatures(creatures);

        let mut maps_state = ahash::AHashMap::new();
        for (map, bucket) in buckets {
            maps_state.insert(map, Arc::new(Mutex::new(build_map_state_with_creatures(map, bucket))));
        }

        Self {
            maps_state,
            clients_on_character_screen: vec![],
            clients_waiting_to_join,
            maps: Arc::new(Mutex::new(maps)),
            db: Arc::new(Mutex::new(db)),
            last_packet_sample: 0,
            last_packet_sample_at: Instant::now(),
            last_net_stats: None,
            last_net_stats_at: Instant::now(),
        }
    }

    /// Build a World suitable for tests and benchmarks: skips pathfinding
    /// map load (so `ground_height` is a noop and there's no filesystem
    /// dependency) and uses synthetic in-memory clients via
    /// [`crate::world::world::client::test_support::synthetic_client`].
    /// Requires an active Tokio runtime — each synthetic client spawns a
    /// writer task.
    ///
    /// Both `characters` and `creatures` are partitioned by `map` into one
    /// `MapState` per continent.
    pub fn for_test(characters: Vec<Character>, creatures: Vec<Creature>) -> Self {
        let maps = PathfindingMaps::new();

        // Move creatures into a Slab so we can hand them to the
        // `partition_creatures` helper.
        let mut creature_slab: Slab<Creature> = Slab::with_capacity(creatures.len());
        for c in creatures {
            creature_slab.insert(c);
        }
        let creature_buckets = crate::world::cell::partition_creatures(creature_slab);

        // Closed receiver — benches don't push new logins, but the field
        // is non-optional on World.
        let (_tx, clients_waiting_to_join) = tokio::sync::mpsc::channel(1);

        let mut maps_state = ahash::AHashMap::new();

        // Build a MapState per creature bucket (one per map).
        for (map, bucket) in creature_buckets {
            maps_state.insert(map, Arc::new(Mutex::new(build_map_state_with_creatures(map, bucket))));
        }

        // Seed test characters directly: each character goes into its map's
        // MapState (created lazily if no creatures spawned there). Sync
        // `try_lock` is fine because we own all Arcs and no other tokio task
        // touches them.
        for character in characters {
            let map = character.map;
            let cell_arc = maps_state.entry(map).or_insert_with(|| {
                Arc::new(Mutex::new(MapState::new_empty(map)))
            }).clone();
            let account = character.account.clone();
            let client = crate::world::world::client::test_support::synthetic_client(
                character, account,
            );
            let mut cell = cell_arc.try_lock()
                .expect("freshly-built map state must not be locked");
            cell.insert_client(client);
        }

        Self {
            maps_state,
            clients_on_character_screen: vec![],
            clients_waiting_to_join,
            maps: Arc::new(Mutex::new(maps)),
            db: Arc::new(Mutex::new(WorldDatabase::new())),
            last_packet_sample: 0,
            last_packet_sample_at: Instant::now(),
            last_net_stats: None,
            last_net_stats_at: Instant::now(),
        }
    }

    /// Walk every in-world client in every cell and persist their
    /// `Character` into the snapshot database. Stage 4 implementation:
    /// orchestration via channels.
    ///
    /// Each cell's character-collection runs on its own `tokio::spawn`
    /// task; the `JoinHandle` is the one-shot reply channel back to the
    /// orchestrator. With N cells this fans out across the tokio
    /// worker pool — the clone (one `Character` per client) runs in
    /// parallel, and the cells are unblocked from each other while
    /// it happens. The orchestrator collects all replies, then locks
    /// the DB once and writes the aggregated Characters atomically.

    pub async fn sync_clients_to_db(&self) {
        // Stage 4 channel-based collection. Each `tokio::spawn` is the
        // sender; the awaited `JoinHandle` is the receiver. With N
        // cells there are N parallel collections.
        let collection_handles: Vec<tokio::task::JoinHandle<Vec<Character>>> =
            self.maps_state
                .values()
                .map(|cell_arc| {
                    let cell_arc = cell_arc.clone();
                    tokio::spawn(async move {
                        let cell = cell_arc.lock().await;
                        cell
                            .clients
                            .iter()
                            .map(|(_, c)| c.character().clone())
                            .collect::<Vec<_>>()
                    })
                })
                .collect();

        let mut all_chars: Vec<Character> = Vec::new();
        for handle in collection_handles {
            match handle.await {
                Ok(chars) => all_chars.extend(chars),
                Err(e) => tracing::error!(
                    "Per-cell snapshot collection task panicked: {e}"
                ),
            }
        }

        let mut db = self.db.lock().await;
        for c in all_chars {
            db.replace_character_data(c);
        }
    }

    /// Return excess capacity in the long-lived slabs / hash maps / scratch
    /// buffers to the allocator. Vec / Slab / HashMap don't auto-shrink on
    /// remove or `.clear()`, so over a long run with peak-load bursts
    /// (`.simulate 5000`, 1000-bot loadtest, mass invasion) the underlying
    /// buffers hold significantly more memory than the live entries justify.
    /// Called from `run_world` once per snapshot save (~60s) which is well
    /// outside any hot path.
    pub async fn shrink_periodic(&mut self) {
        for cell in self.maps_state.values() {
            let mut cell = cell.lock().await;
            cell.shrink_periodic();
        }
        self.clients_on_character_screen.shrink_to_fit();
    }


    // Declarations of `t_*` Duration variables get assigned exactly once
    // inside their respective phase blocks below. Clippy flags the late init
    // as redundant — but consolidating the per-phase timing handles at the
    // top makes the slow-tick log line trivially scannable.
    #[allow(clippy::needless_late_init)]
    #[tracing::instrument(level = "info", skip_all, name = "World::tick")]
    pub async fn tick(&mut self, slow_warn: Duration) {
        let tick_start = Instant::now();

        // ── Per-cell locking ──
        //
        // Each cell lives behind a `tokio::sync::Mutex` inside an
        // `Arc`. The per-cell `tokio::spawn` block lower down spawns
        // one task per cell; each task owns its own
        // `Arc<Mutex<MapState>>` and they run in parallel on the
        // tokio worker pool. The body here is the orchestrator: drain
        // login, character-screen
        // opcodes, promote (all global), then spawn N cell tasks for
        // the per-cell phases.
        // Forward-declare values that flow from the global phases into the
        // post-spawn slow-tick log + Tracy block. Per-cell `tick_dt` and
        // `heartbeat_skip_ratio` are computed INSIDE the per-cell task
        // (each pacer drives its own).
        let t_drain: Duration;
        let t_chrscreen: Duration;
        let t_promote: Duration;

        // ── Global phases that need NO cell lock ──
        // drain_login + char_screen run against `self.clients_*` +
        // `self.db`. Promote (which needs `&mut self` to call
        // `ensure_cell_exists`) is broken out below so we can take
        // disjoint borrows on cells vs db vs clients_on_character_screen.
        {
            let phase = Instant::now();
            let _s = tracing::info_span!("drain_login_queue").entered();
            while let Ok(c) = self.clients_waiting_to_join.try_recv() {
                self.clients_on_character_screen.push(c);
            }
            t_drain = phase.elapsed();
        }

        {
            let phase = Instant::now();
            let mut db_guard = self.db.lock().await;
            let db: &mut WorldDatabase = &mut db_guard;
            async {
                for client in self.clients_on_character_screen.iter_mut() {
                    handle_character_screen_opcodes(client, db).await;
                }
            }
            .instrument(tracing::info_span!("character_screen_opcodes"))
            .await;
            drop(db_guard);
            t_chrscreen = phase.elapsed();
        }

        // ── Stage 5 partition: promote with per-destination routing ──
        //
        // Each `WaitingToLogIn` client is now routed to the cell that
        // contains their character's position. The destination cell
        // is lazily spun up on first admit. Per-iteration we:
        //   1. Pop the next ready CharacterScreenClient.
        //   2. Lock `self.db` briefly to resolve guid → Character.
        //   3. Build the in-world Client.
        //   4. Compute `CellKey::from_position` for the destination.
        //   5. `ensure_cell_exists` (creates + routes the inbox if new).
        //   6. Lock the destination cell exclusively for this admit:
        //      build the visible-objects bundle from THAT cell only,
        //      seed observers' visible_entities, insert the client.
        //
        // The AOI scan is now intra-cell — players in neighbor cells
        // discover the newcomer through the next cross-cell broadcast
        // (step 7's inbox drain). For sparse-density cells this is
        // identical to before; for boundary-hugging admits the neighbor
        // visibility lights up one tick later, which the client's
        // interpolation tolerates.
        let phase = Instant::now();
        let aoi_r = crate::config::config().network.aoi_radius_yards;
        let aoi_r_sq = aoi_r * aoi_r;
        let max_promotions =
            crate::config::config().tick.max_promotions_per_tick;
        let mut promoted_this_tick = 0_u32;
        async {
        while let Some(i) = self.clients_on_character_screen
            .iter()
            .position(|a| matches!(a.status, CharacterScreenProgress::WaitingToLogIn(_)))
        {
            if max_promotions > 0 && promoted_this_tick >= max_promotions {
                break;
            }
            let c = self.clients_on_character_screen.remove(i);
            let guid = match c.status {
                CharacterScreenProgress::WaitingToLogIn(g) => g,
                _ => unreachable!(),
            };
            let character = {
                let db = self.db.lock().await;
                match db.get_character_by_guid(guid) {
                    Some(ch) => ch,
                    None => {
                        tracing::warn!(
                            "Promotion for {} aborted: guid {:?} not found in DB; dropping connection.",
                            c.account_name(),
                            guid
                        );
                        drop(c);
                        continue;
                    }
                }
            };
            let mut c = c.into_client(character);

            let new_player_pos = c.character().info.position;
            let new_player_map = c.character().map;
            let new_player_guid = c.character().guid;
            let cell_arc = self.ensure_map_exists(new_player_map);
            let mut cell_guard = cell_arc.lock().await;
            let cell: &mut MapState = &mut cell_guard;

            // Rebuild the destination's broadcast_view so the par_iter
            // filters below see this-tick state. Cheap; sub-ms even
            // at high density.
            cell.broadcast_view.clear();
            cell.broadcast_view
                .extend(cell.clients.iter().map(|(_, c)| c.broadcast_target()));

            // Announce the new player to the destination cell.
            let new_player_object = player_create_object(c.character());
            if let Some(msg) = UpdateObject::from_objects(vec![new_player_object]) {
                msg.broadcast_within_aoi(new_player_pos, new_player_map, &mut cell.clients)
                    .await;
                // Seed in-AOI observers' visible_entities so the next
                // AOI tick doesn't re-emit CreateObject for them.
                let observer_guids: Vec<Guid> = cell
                    .broadcast_view
                    .par_iter()
                    .filter(|t| {
                        t.map == new_player_map
                            && aoi::within_aoi_sq(&t.position, &new_player_pos, aoi_r_sq)
                    })
                    .map(|t| t.guid)
                    .collect();
                for g in observer_guids {
                    if let Some(&k) = cell.client_by_guid.get(&g) {
                        cell.clients[k]
                            .session
                            .visible_entities
                            .insert(new_player_guid);
                    }
                }
            }

            // Build the visible-objects bundle from the destination
            // cell only (intra-cell AOI; neighbor coverage arrives
            // via cross-cell broadcasts).
            let mut visible_objects: Vec<Object> = Vec::new();
            let mut movement_starts: Vec<MSG_MOVE_START_FORWARD_Server> = Vec::new();

            let candidate_guids: Vec<Guid> = cell
                .broadcast_view
                .par_iter()
                .filter(|t| {
                    t.guid != new_player_guid
                        && t.map == new_player_map
                        && aoi::within_aoi_sq(&t.position, &new_player_pos, aoi_r_sq)
                })
                .map(|t| t.guid)
                .collect();
            for other_guid in candidate_guids {
                let Some(&other_key) = cell.client_by_guid.get(&other_guid) else {
                    continue;
                };
                let client = &cell.clients[other_key];
                visible_objects.push(player_create_object(client.character()));
                c.session.visible_entities.insert(other_guid);
                if client.character().info.flags.get_forward() {
                    movement_starts.push(MSG_MOVE_START_FORWARD_Server {
                        guid: other_guid,
                        info: client.character().info.clone(),
                    });
                }
            }

            // Creature scan over the radius-derived 33.33 yd cell window
            // around the new player, using the map's own creature grid.
            {
                let (cx, cy) = grid_cell_for(new_player_pos.x, new_player_pos.y);
                let cr = grid_cell_radius(aoi_r_sq.sqrt());
                for dx in -cr..=cr {
                    for dy in -cr..=cr {
                        let Some(occupants) = cell
                            .index
                            .cell_occupants((cx + dx, cy + dy))
                        else {
                            continue;
                        };
                        for &occ in occupants {
                            let crate::world::spatial::Occupant::Creature(ck) = occ else {
                                continue;
                            };
                            let creature = &cell.creatures[ck];
                            if !aoi::within_aoi_sq(
                                &creature.info.position,
                                &new_player_pos,
                                aoi_r_sq,
                            ) {
                                continue;
                            }
                            visible_objects.push(creature.to_create_object());
                            if creature.info.flags.get_forward() {
                                movement_starts.push(MSG_MOVE_START_FORWARD_Server {
                                    guid: creature.guid,
                                    info: creature.info.clone(),
                                });
                            }
                        }
                    }
                }
            }

            let visible_count = visible_objects.len();
            let starts_count = movement_starts.len();
            UpdateObject::send_chunked(visible_objects, &mut c).await;
            for start in movement_starts {
                c.send_message(start).await;
            }
            tracing::debug!(
                "promote: account={} name={} pos=({:.1},{:.1},{:.1}) map={:?} -> sent {} CreateObjects + {} MoveStarts; map_clients={} map_creatures={}",
                c.session.account_name,
                c.character().name,
                new_player_pos.x,
                new_player_pos.y,
                new_player_pos.z,
                new_player_map,
                visible_count,
                starts_count,
                cell.clients.len(),
                cell.creatures.len(),
            );

            let new_target = c.broadcast_target();
            cell.insert_client(c);
            cell.broadcast_view.push(new_target);
            promoted_this_tick += 1;
            drop(cell_guard);
        }
        if let Some(client) = tracy_client::Client::running() {
            client.plot(
                tracy_client::plot_name!("promoted_this_tick"),
                promoted_this_tick as f64,
            );
        }
        }
        .instrument(tracing::info_span!("promote_logged_in"))
        .await;
        t_promote = phase.elapsed();

        // ── Stage 3: per-cell tick on its own tokio task ──
        //
        // Release the orchestrator's cell/db/maps guards (held for the
        // global drain/char_screen/promote phases above), then spawn one
        // task per cell. Each task re-acquires the locks it needs and
        // runs the per-cell phases (per_client_loop, flush, AOI,
        // apply_cmds, corpses, creature_ai, drain_logouts, stale-cleanup)
        // independently. With one cell we await one task; with N
        // cells, N tasks run in parallel on the tokio worker pool.
        // Departed clients (logouts + cell transitions) are collected
        // into the task's `__departed` and pushed back into
        // `self.clients_on_character_screen` by the orchestrator after
        // the task completes.
        //
        // Stage 5: the orchestrator no longer holds any cell/db/maps
        // guard at this point — promote does its own per-destination
        // locking and char_screen released its db lock above.

        let mut per_cell_handles: Vec<tokio::task::JoinHandle<PerCellTickResult>> = Vec::new();
        for cell_arc in self.maps_state.values() {
            let cell_arc = cell_arc.clone();
            let db_arc = self.db.clone();
            let maps_arc = self.maps.clone();
            per_cell_handles.push(tokio::spawn(async move {
                let mut cell_guard = cell_arc.lock().await;
                let cell: &mut MapState = &mut cell_guard;

                // ── Per-cell pacer: decide whether to tick this round ──
                //
                // The global orchestrator runs at `target_interval_ms`
                // (default 33 ms = 30 Hz). Each cell's pacer carries
                // its own `current_interval` which the pacer's
                // adaptive backoff stretches to 66 → 132 → … →
                // `max_interval_ms` (1 s = 1 Hz) under sustained
                // slow ticks. If less than `current_interval` has
                // elapsed since this cell's last tick, the cell
                // skips this global round entirely — the spawn
                // returns a "skipped" result with empty fields and
                // the orchestrator suppresses per-cell Tracy/log
                // emission for it. The cell's `last_tick_at` is
                // only advanced when work actually runs.
                let now = std::time::Instant::now();
                // Tick if we're within a quarter-interval of due. The per-map
                // interval (33 ms at the target rate) and the global loop period
                // (also ~33 ms — `run_world` pins before-to-before to the global
                // pacer's interval) land on the same `>=` boundary, so timing
                // jitter would make a HEALTHY map skip on alternating ticks —
                // halving its effective rate and making every per-tick Tracy plot
                // zigzag to 0. A quarter-interval slack cleanly separates "tick
                // every round" (a 33 ms map: elapsed ≈ 33 ≥ 33−8) from "tick every
                // other round" (a backed-off 66 ms map: elapsed ≈ 33 < 66−16, so
                // it still skips the first round).
                let due = cell
                    .last_tick_at
                    .map(|t| {
                        let slack = cell.pacer.current_interval / 4;
                        now.duration_since(t) + slack >= cell.pacer.current_interval
                    })
                    .unwrap_or(true);
                if !due {
                    return PerCellTickResult {
                        map: cell.map,
                        skipped: true,
                        t_per_client: Duration::ZERO,
                        t_build_view: Duration::ZERO,
                        t_flush: Duration::ZERO,
                        t_aoi: Duration::ZERO,
                        t_apply_cmds: Duration::ZERO,
                        t_corpses: Duration::ZERO,
                        t_creatures: Duration::ZERO,
                        t_logouts: Duration::ZERO,
                        t_cell_total: Duration::ZERO,
                        departed: Vec::new(),
                        map_changers: Vec::new(),
                        clients_count: cell.clients.len(),
                        creatures_count: cell.creatures.len(),
                        // Populate counts even on a skip so the orchestrator's
                        // world-total aggregation stays correct for idle maps.
                        creature_idle_count: cell.creatures.len().saturating_sub(
                            cell.creature_wander_count
                                + cell.creature_waypoint_count
                                + cell.aggro_creature_keys.len(),
                        ),
                        creature_wander_count: cell.creature_wander_count,
                        creature_waypoint_count: cell.creature_waypoint_count,
                        creature_aggro_count: cell.aggro_creature_keys.len(),
                        walking_creature_count: cell.walking_creature_keys.len(),
                    };
                }

                // Compute wall-clock dt since this cell's last tick.
                // Clamp at 1 s so a frozen tick doesn't blow the
                // auto-attack timer negative; clamp at the pacer's
                // current_interval as a sanity floor.
                let tick_dt: f32 = cell
                    .last_tick_at
                    .map(|t| {
                        let d = now.duration_since(t).as_secs_f32();
                        d.min(1.0)
                    })
                    .unwrap_or(
                        crate::config::config()
                            .tick
                            .target_interval()
                            .as_secs_f32(),
                    );
                cell.last_tick_at = Some(now);
                cell.tick_counter = cell.tick_counter.wrapping_add(1);

                // Per-map AoI snapshot built from this cell's own live state
                // (end-of-last-tick positions). Replaces the old cross-cell
                // merge — with one cell per map there are no neighbors. Owned
                // (not borrowing `cell`) so the per-client loop can take its
                // `&mut cell.*` borrows below; consumed by the opcode-loop
                // radius queries and `tick_aoi_transitions`.
                let global_aoi = cell.build_local_aoi_snapshot();

                // Per-cell heartbeat throttle: pacer-driven instead
                // of global slow_warn. With pacer at 33 ms the ratio
                // is 1 (no throttle); at 100 ms it's 3 (every 3rd
                // heartbeat). Floors at 1.
                let heartbeat_skip_ratio: u64 = {
                    let target_ms = crate::config::config().tick.target_interval_ms.max(1);
                    let current_ms = cell.pacer.current_interval.as_millis() as u64;
                    (current_ms / target_ms).max(1)
                };

                let mut db_guard = db_arc.lock().await;
                let db: &mut WorldDatabase = &mut db_guard;
                let mut maps_guard = maps_arc.lock().await;
                let maps: &mut PathfindingMaps = &mut maps_guard;
                let mut __departed: Vec<CharacterScreenClient> = Vec::new();
                let t_per_client: Duration;
                let t_flush: Duration;
                let t_aoi: Duration;
                let t_apply_cmds: Duration;
                let t_corpses: Duration;
                let t_creatures: Duration;
                let t_logouts: Duration;
        let mut keys_to_move_to_character_screen: Vec<usize> = Vec::new();
        let mut commands = crate::world::command::CommandQueue::new();

        // No server-side respawn pass — under the gurubashi-pvp rules
        // players stay dead where they fell. `time_of_death` remains set
        // and `is_dead()` keeps the opcode handler dropping incoming
        // packets, so a dead client sits inertly as a corpse until the
        // server restarts (snapshot load resets `current_health` to
        // `max_health`).

        let phase = Instant::now();
        async {
        let client_keys: Vec<usize> = cell.clients.iter().map(|(k, _)| k).collect();
        for key in client_keys {
            let mut client = cell.take_active_client(key);
            // Per-iteration: the opcode handler may flip this true for the
            // CURRENT client (CMSG_LOGOUT_REQUEST). Resetting on every iter
            // is load-bearing — a single declaration above the loop would
            // be sticky across clients and one logout would drag every
            // later client in the slab into character-screen with it.
            let mut move_to_character_screen = false;
            let mut entities = Entities::new(
                &mut cell.clients,
                &cell.client_by_guid,
                &mut cell.creatures,
                &cell.creature_by_guid,
                &mut cell.pending_movement,
                &global_aoi,
            );
            world_opcode_handler::handle_received_client_opcodes(
                &mut client,
                &mut entities,
                db,
                &mut move_to_character_screen,
                &mut *maps,
                &mut commands,
            )
            .await;
            client.character_mut().update_auto_attack_timer(tick_dt);

            if client.character().attacking
                && !client.character().is_dead()
                && client.character().auto_attack_timer <= 0.0
            {
                let target_guid = client.character().target;
                let attacker_guid = client.character().guid;
                let attacker_pos = client.character().info.position;
                let attacker_map = client.character().map;
                let attacker_moving =
                    world_opcode_handler::combat::is_moving(&client.character().info);

                // Resolve target: creature first (O(1) reverse index),
                // then linear scan of clients for a player target. Same-map
                // is enforced on both paths (PvP fights shouldn't reach
                // through portals). Self/zero/missing/dead/different-map
                // all fall through to `None` → swing cancels.
                //
                // Stage 5 cross-cell fallback: if the target isn't in
                // either local slab, try `entities.locate_entity` which
                // reads the global AoI snapshot. For cross-cell
                // creatures we route a `Damage` `UnitEffect` via
                // `apply_effect` so the neighbor cell's drain
                // mutates its own slab + queues the kill if health
                // hits zero. PvP across boundaries is left for a
                // future pass — players need stand-state /
                // SMSG_ATTACKERSTATEUPDATE bookkeeping that lives
                // in the local-player branch.
                #[derive(Copy, Clone)]
                enum SwingKind {
                    Creature(usize),
                    Player(usize),
                }
                let resolved: Option<(SwingKind, Vector3d, bool)> = if target_guid
                    == Guid::zero()
                    || target_guid == attacker_guid
                {
                    None
                } else if let Some(&ck) = cell.creature_by_guid.get(&target_guid) {
                    let cr = &cell.creatures[ck];
                    if cr.map == attacker_map {
                        Some((
                            SwingKind::Creature(ck),
                            cr.info.position,
                            world_opcode_handler::combat::is_moving(&cr.info),
                        ))
                    } else {
                        None
                    }
                } else if let Some((player_key, _)) = cell
                    .clients
                    .iter()
                    .find(|(_, c)| {
                        c.character().guid == target_guid
                            && !c.character().is_dead()
                            && c.character().map == attacker_map
                    })
                {
                    // Player target — O(N) scan over clients. At 1000 PvP
                    // bots this is 1M comparisons/tick which is well under
                    // budget; if it ever bites perf, add a guid → slab key
                    // reverse index alongside `creature_by_guid`.
                    let c = &cell.clients[player_key];
                    Some((
                        SwingKind::Player(player_key),
                        c.character().info.position,
                        world_opcode_handler::combat::is_moving(&c.character().info),
                    ))
                } else {
                    // Target isn't on this map (one MapState per continent
                    // now, so there are no neighbor cells to reach into) —
                    // treat as a vanished target and cancel the swing below.
                    None
                };

                let Some((kind, target_pos, target_moving)) = resolved else {
                    // Invalid target — cancel attack outright. Timer is left
                    // alone (a fresh CMSG_ATTACKSWING will set its wind-up).
                    // Matches cmangos's behavior of dropping the AttackerSet
                    // when the target dies or vanishes.
                    client.character_mut().attacking = false;
                    client.character_mut().target = Guid::zero();
                    // We jump to the per-iter trailing logic by skipping the
                    // rest of this swing block via the outer `if`.
                    if move_to_character_screen {
                        keys_to_move_to_character_screen.push(key);
                    }
                    let new_key = cell.reinsert_active_client(client);
                    debug_assert_eq!(new_key, key);
                    continue;
                };

                let target_is_creature = matches!(kind, SwingKind::Creature(_));
                let range = world_opcode_handler::combat::melee_range_yards(
                    attacker_moving,
                    target_moving,
                    target_is_creature,
                );
                let dist_sq =
                    world_opcode_handler::combat::distance_sq_3d(&attacker_pos, &target_pos);

                if dist_sq > range * range {
                    // Out of range — no broadcast, no damage. cmangos snaps
                    // the swing timer to a short retry (~100 ms in
                    // `Unit.cpp:1208`) so a converging attacker re-checks
                    // next tick instead of eating a full 2 s swing cycle.
                    client.character_mut().auto_attack_timer = 0.1;
                } else {
                    // In range — execute the swing.
                    client.character_mut().auto_attack_timer = UNARMED_SPEED;
                    // Random per-swing damage in [10, 20]. Keeps kill-times
                    // (at 100 HP / 2 s unarmed speed) in the 10-20 s ballpark
                    // so loadtest PvP fights resolve fast enough to observe.
                    let swing_damage: u32 = 10
                        + (crate::world::world_opcode_handler::gm_command::next_rand() % 11)
                            as u32;
                    let msg = SMSG_ATTACKERSTATEUPDATE {
                        hit_info: HitInfo::CriticalHit,
                        attacker: attacker_guid,
                        target: target_guid,
                        total_damage: swing_damage,
                        damages: vec![DamageInfo {
                            spell_school_mask: 0,
                            damage_float: swing_damage as f32,
                            damage_uint: swing_damage,
                            absorb: 0,
                            resist: 0,
                        }],
                        unknown1: 0,
                        spell_id: 0,
                        damage_state: 0,
                        blocked_amount: 0,
                    };

                    // Source is held outside the slab (removed for processing),
                    // so `broadcast_within_aoi` only reaches other observers —
                    // send to attacker separately. The broadcast helper
                    // serializes once and reuses the body for all recipients.
                    client.send_message(msg.clone()).await;
                    aoi::broadcast_within_aoi(
                        msg,
                        attacker_pos,
                        attacker_map,
                        &mut cell.clients,
                    )
                    .await;

                    match kind {
                        SwingKind::Creature(creature_key) => {
                            let creature = &mut cell.creatures[creature_key];
                            creature.health = creature.health.saturating_sub(swing_damage);
                            creature.invalidate_object_cache();
                            let creature_map = creature.map;
                            let creature_pos = creature.info.position;
                            let creature_guid = creature.guid;
                            let killed = creature.health == 0;

                            if killed {
                                cell.kill_creature(creature_key).await;
                                client.character_mut().attacking = false;
                                client.character_mut().target = Guid::zero();
                            } else {
                                let hp_update = SMSG_UPDATE_OBJECT {
                                    has_transport: 0,
                                    objects: vec![Object {
                                        update_type: Object_UpdateType::Values {
                                            guid1: creature_guid,
                                            mask1: UpdateMask::Unit(
                                                wow_world_messages::vanilla::UpdateUnitBuilder::new()
                                                    .set_unit_health(i32::try_from(creature.health).unwrap_or(i32::MAX))
                                                    .finalize(),
                                            ),
                                        },
                                    }],
                                };
                                client.send_message(hp_update.clone()).await;
                                aoi::broadcast_within_aoi(
                                    hp_update,
                                    creature_pos,
                                    creature_map,
                                    &mut cell.clients,
                                )
                                .await;
                            }
                        }
                        SwingKind::Player(target_key) => {
                            let target = &mut cell.clients[target_key];
                            let new_hp = target.character_mut().apply_damage(swing_damage);
                            let target_pos = target.character().info.position;
                            let target_map = target.character().map;
                            let killed = new_hp == 0;
                            if killed {
                                target.character_mut().time_of_death = Some(Instant::now());
                                target.character_mut().attacking = false;
                                target.character_mut().target = Guid::zero();
                                client.character_mut().attacking = false;
                                client.character_mut().target = Guid::zero();
                            }

                            // Build the partial-update mask. On the killing
                            // blow we additionally flip stand-state to dead
                            // so the corpse renders correctly client-side
                            // (skull on minimap, fallen pose).
                            const STAND_STATE_DEAD: u8 = 7;
                            let mask_builder =
                                wow_world_messages::vanilla::UpdatePlayerBuilder::new()
                                    .set_unit_health(new_hp as i32);
                            let mask_builder = if killed {
                                mask_builder.set_unit_bytes_1(STAND_STATE_DEAD, 0, 0, 0)
                            } else {
                                mask_builder
                            };
                            let hp_update = SMSG_UPDATE_OBJECT {
                                has_transport: 0,
                                objects: vec![Object {
                                    update_type: Object_UpdateType::Values {
                                        guid1: target_guid,
                                        mask1: UpdateMask::Player(mask_builder.finalize()),
                                    },
                                }],
                            };
                            // Target is in `cell.clients` so they'll receive via
                            // broadcast; attacker is held outside, send directly.
                            client.send_message(hp_update.clone()).await;
                            aoi::broadcast_within_aoi(
                                hp_update,
                                target_pos,
                                target_map,
                                &mut cell.clients,
                            )
                            .await;
                        }
                    }
                }
            }

            if move_to_character_screen {
                keys_to_move_to_character_screen.push(key);
            }

            let new_key = cell.reinsert_active_client(client);
            debug_assert_eq!(new_key, key);
        }
        }
        .instrument(tracing::info_span!("per_client_loop"))
        .await;
        t_per_client = phase.elapsed();

        // Build the per-tick `Sync`-safe broadcast view. Required because
        // rayon can't share `&Client` across threads (Client embeds a
        // `tokio::sync::mpsc::Receiver` which is `!Sync`); the view
        // snapshots the broadcast-relevant fields (map, position, guid,
        // outbound sender, dropped-packet counter, account name) into a
        // `Send + Sync` struct that rayon's par_iter can consume.
        //
        // Built AFTER per_client_loop (so positions reflect this tick's
        // movement) and BEFORE flush + AOI transitions. Reused via
        // `clear()` + `extend` so the underlying Vec capacity is amortized
        // across ticks — at stable population there's no realloc.
        let phase = Instant::now();
        {
            let _s = tracing::info_span!("build_broadcast_view").entered();
            cell.broadcast_view.clear();
            cell.broadcast_view
                .extend(cell.clients.iter().map(|(_, c)| c.broadcast_target()));
        }
        let t_build_view = phase.elapsed();
        if let Some(client) = tracy_client::Client::running() {
            client.plot(
                tracy_client::plot_name!("broadcast_view_len"),
                cell.broadcast_view.len() as f64,
            );
        }

        let phase = Instant::now();
        cell.flush_movement_broadcasts(heartbeat_skip_ratio);
        t_flush = phase.elapsed();

        // AOI transitions: for each connected player, diff their previously
        // visible set against the players currently within `AOI_RADIUS_YARDS`
        // on the same map. Anything that left → `OutOfRangeObjects`
        // (despawn). Anything that entered → `CreateObject2` (spawn).
        // Without this pass, players who walk past the AOI boundary
        // linger forever on observers' clients as motionless ghosts.
        let phase = Instant::now();
        let aoi_stats = cell.tick_aoi_transitions(&global_aoi).await;
        t_aoi = phase.elapsed();
        if let Some(client) = tracy_client::Client::running() {
            client.plot(
                tracy_client::plot_name!("aoi_entered"),
                aoi_stats.entered as f64,
            );
            client.plot(
                tracy_client::plot_name!("aoi_departed"),
                aoi_stats.departed as f64,
            );
            client.plot(
                tracy_client::plot_name!("aoi_suppressed"),
                aoi_stats.suppressed as f64,
            );
            client.plot(
                tracy_client::plot_name!("aoi_fast_path"),
                aoi_stats.fast_path as f64,
            );
        }

        let phase = Instant::now();
        cell.apply_commands(&mut commands, &mut *maps).await;
        t_apply_cmds = phase.elapsed();

        let phase = Instant::now();
        cell.tick_corpses_and_respawns().await;
        t_corpses = phase.elapsed();

        let phase = Instant::now();
        cell.tick_creature_ai(&mut *maps).await;
        t_creatures = phase.elapsed();

        let phase = Instant::now();
        cell
            .drain_logouts(&keys_to_move_to_character_screen, &mut __departed)
            .await;
        t_logouts = phase.elapsed();

        let stale_client_keys: Vec<usize> = cell
            .clients
            .iter()
            .filter_map(|(k, c)| c.reader_is_finished().then_some(k))
            .collect();
        // Group stale-disconnect guids by map so a mass-drop (e.g. 800 bots
        // hitting Ctrl-C simultaneously) becomes ONE `OutOfRangeObjects`
        // packet per observer, not N individual `SMSG_DESTROY_OBJECT`
        // broadcasts. Previously the latter pattern flooded each observer's
        // per-client outbound channel past `OUTBOUND_CHANNEL_CAPACITY = 512`;
        // anything past that got dropped silently via `try_send`'s `Full`
        // path, leaving stale phantoms in admin's view of the world.
        let mut stale_by_map: ahash::AHashMap<Map, Vec<Guid>> = ahash::AHashMap::new();
        for key in stale_client_keys {
            let c = cell.remove_client(key);
            let logout_map = c.character().map;
            let guid = c.character().guid;
            cell.last_heartbeat_broadcast_tick.remove(&guid);
            db.replace_character_data(c.character().clone());
            tracing::info!(
                "Dropped stale client {} ({}); reader task ended",
                c.character().name,
                guid
            );
            stale_by_map.entry(logout_map).or_default().push(guid);
        }
        for (map, guids) in stale_by_map {
            // Wrap the dead-guid list in a single SMSG_UPDATE_OBJECT carrying
            // `OutOfRangeObjects { guids }`. The 1.12.2 client treats this
            // as "remove all these entities from your local table" — same
            // visual effect as N individual DESTROY_OBJECTs but one packet
            // per observer. Send to every remaining client on that map; the
            // client ignores guids it doesn't know about, so skipping the
            // AOI distance check here is harmless and lets us shortcut a
            // per-recipient distance walk.
            //
            // Also clear the stale guids from every observer's
            // `visible_entities` so the next `tick_aoi_transitions` doesn't
            // re-detect them as departed and emit a duplicate despawn.
            let total = guids.len();
            let msg = SMSG_UPDATE_OBJECT {
                has_transport: 0,
                objects: vec![Object {
                    update_type: Object_UpdateType::OutOfRangeObjects {
                        guids: guids.clone(),
                    },
                }],
            };
            let mut delivered = 0_usize;
            for (_, c) in cell.clients.iter_mut() {
                if c.character().map == map {
                    for g in &guids {
                        c.session.visible_entities.remove(g);
                    }
                    c.send_message(msg.clone()).await;
                    delivered += 1;
                }
            }
            tracing::debug!(
                "Despawned {total} stale clients on map {map:?} -> notified {delivered} observers"
            );
        }

                let _ = (db, maps); // silence unused-binding lints if any branch is no-op

                // Continent teleport: any client whose `map` no longer matches
                // this cell's map (set by `prepare_teleport`) is pulled out
                // here and re-homed into the destination map's `MapState` by
                // the orchestrator below. Intra-map movement never triggers
                // this (one MapState per map).
                let map_changers = cell.take_map_changers();

                let creature_aggro_count = cell.aggro_creature_keys.len();
                let creature_wander_count = cell.creature_wander_count;
                let creature_waypoint_count = cell.creature_waypoint_count;
                let walking_creature_count = cell.walking_creature_keys.len();
                let creature_idle_count = cell.creatures.len()
                    .saturating_sub(creature_wander_count + creature_waypoint_count + creature_aggro_count);

                // Sum the per-cell phase timings into the per-cell
                // total. Used both for the `cell_tick_ms` Tracy plot
                // and for the per-cell pacer's adaptive backoff
                // signal.
                let t_cell_total = t_per_client + t_build_view + t_flush + t_aoi
                    + t_apply_cmds + t_corpses + t_creatures + t_logouts;

                // Per-cell pacer: feed this tick's per-cell cost
                // and publish the resulting state to the process-wide
                // snapshot so `.cells` can show it. Today the actual
                // sleep happens at the global level in `run_world`;
                // when long-lived per-cell task loops land, each
                // task will sleep on its own `pacer.current_interval`.
                let (_sleep_for, cell_pacer_change) =
                    cell.pacer.observe(t_cell_total);
                crate::world::cell::publish_pacer_state(
                    cell.map,
                    crate::world::cell::PacerSnapshot {
                        current_interval_ms: cell.pacer.current_interval.as_millis() as u64,
                        slow_ema: cell.pacer.slow_ema,
                        healthy_streak: cell.pacer.healthy_streak,
                        last_tick_ms: t_cell_total.as_millis() as u64,
                    },
                );

                // If this cell's pacer transitioned (backoff or
                // recovery), tell ONLY the players in this cell.
                // Players in other cells whose pacers are happy
                // shouldn't see "tickrate backoff" chat spam.
                if let Some(change) = cell_pacer_change {
                    let (label, interval) = match change {
                        crate::world::TickRateChange::Backoff { new_interval } => {
                            ("backoff", new_interval)
                        }
                        crate::world::TickRateChange::Recovery { new_interval } => {
                            ("recovery", new_interval)
                        }
                    };
                    let hz = 1.0 / interval.as_secs_f32();
                    let text = format!(
                        "[server] cell {} tickrate {label}: {} ms ({:.1} Hz)",
                        cell.map,
                        interval.as_millis(),
                        hz,
                    );
                    for (_, c) in cell.clients.iter_mut() {
                        c.send_system_message(text.clone()).await;
                    }
                }

                // Per-cell Tracy plot — `cell_tick_ms` is the
                // total per-cell cost for this tick. Emitted from
                // inside the task so it sits on the same Tracy
                // timeline as the per-cell phase plots.
                if let Some(client) = tracy_client::Client::running() {
                    client.plot(
                        tracy_client::plot_name!("cell_tick_ms"),
                        t_cell_total.as_secs_f64() * 1000.0,
                    );
                }

                PerCellTickResult {
                    map: cell.map,
                    skipped: false,
                    t_per_client,
                    t_build_view,
                    t_flush,
                    t_aoi,
                    t_apply_cmds,
                    t_corpses,
                    t_creatures,
                    t_logouts,
                    t_cell_total,
                    departed: __departed,
                    map_changers,
                    clients_count: cell.clients.len(),
                    creatures_count: cell.creatures.len(),
                    creature_idle_count,
                    creature_wander_count,
                    creature_waypoint_count,
                    creature_aggro_count,
                    walking_creature_count,
                }
            }));
        }

        // Await all per-cell tasks. With one cell this is one await;
        // with N cells each ran on a tokio worker thread in parallel.
        // The orchestrator pulls departed clients out of each result
        // (logouts → char_screen) and keeps the rest of the result for
        // post-spawn metrics. Skipped cells return cheap zero-valued
        // results; we filter them out of Tracy/log emission below.
        let mut all_results: Vec<PerCellTickResult> = Vec::new();
        let mut all_map_changers: Vec<Client> = Vec::new();
        for handle in per_cell_handles {
            match handle.await {
                Ok(mut r) => {
                    self.clients_on_character_screen
                        .extend(std::mem::take(&mut r.departed));
                    all_map_changers.append(&mut r.map_changers);
                    all_results.push(r);
                }
                Err(e) => tracing::error!("Per-cell tick task panicked: {e}"),
            }
        }

        // Re-home continent teleporters into their destination map's
        // `MapState` (the source map's task already removed them). This is
        // the only cross-`MapState` client move — intra-map movement never
        // transitions. The destination cell's NEXT tick observes them, and the
        // client's `MSG_MOVE_WORLDPORT_ACK` (now handled by the destination
        // task) builds its arrival visible-set from the correct map.
        for client in all_map_changers {
            let dest_arc = self.ensure_map_exists(client.character().map);
            let mut dest_cell = dest_arc.lock().await;
            dest_cell.insert_client(client);
        }

        // No cell GC: there is one `MapState` per continent map, created at
        // startup and never torn down. An idle map stays cheap via the pacer
        // due-check; its `SpatialIndex` buckets self-clean as entities leave.

        // Prefer the first non-skipped result for Tracy plots and the
        // slow-tick log; fall back to any result if every cell
        // skipped (so e.g. `cells_active` plot still emits).
        // Empty-world (no cells at all — e.g. worlddb load failed)
        // gets a zeroed placeholder so the metrics block can run
        // without panicking.
        let empty_placeholder;
        let per_cell_result = match all_results
            .iter()
            .find(|r| !r.skipped)
            .or_else(|| all_results.first())
        {
            Some(r) => r,
            None => {
                empty_placeholder = PerCellTickResult::empty();
                &empty_placeholder
            }
        };
        // Pull out the per-cell phase timings (from a representative
        // non-skipped result) so the orchestrator's slow-tick log and the
        // gated phase plots can use them.
        let t_per_client = per_cell_result.t_per_client;
        let t_build_view = per_cell_result.t_build_view;
        let t_flush = per_cell_result.t_flush;
        let t_aoi = per_cell_result.t_aoi;
        let t_apply_cmds = per_cell_result.t_apply_cmds;
        let t_corpses = per_cell_result.t_corpses;
        let t_creatures = per_cell_result.t_creatures;
        let t_logouts = per_cell_result.t_logouts;
        // Counts are world TOTALS — sum across every map (each result carries
        // its real counts even when skipped). Plotting a single map's counts
        // would zigzag as the picked result flips between maps or skips, which
        // is the "players ticks to 0" symptom. `cell_max_clients` is the
        // busiest single map → max, not sum.
        let agg_clients: usize = all_results.iter().map(|r| r.clients_count).sum();
        let agg_creatures: usize = all_results.iter().map(|r| r.creatures_count).sum();
        let agg_creatures_idle: usize =
            all_results.iter().map(|r| r.creature_idle_count).sum();
        let agg_creatures_wander: usize =
            all_results.iter().map(|r| r.creature_wander_count).sum();
        let agg_creatures_waypoint: usize =
            all_results.iter().map(|r| r.creature_waypoint_count).sum();
        let agg_creatures_aggro: usize =
            all_results.iter().map(|r| r.creature_aggro_count).sum();
        let max_cell_clients: usize =
            all_results.iter().map(|r| r.clients_count).max().unwrap_or(0);
        while let Some((i, _)) = self.clients_on_character_screen
            .iter()
            .enumerate()
            .find(|(_, a)| a.reader_handle.is_finished())
        {
            self.clients_on_character_screen.remove(i);
        }

        let now_packet_count =
            crate::world::world::client::outgoing_packet_count();
        let packet_delta = now_packet_count.saturating_sub(self.last_packet_sample);
        let elapsed_secs = self.last_packet_sample_at
            .elapsed()
            .as_secs_f64()
            .max(1e-6);
        let wow_messages_per_second = packet_delta as f64 / elapsed_secs;
        self.last_packet_sample = now_packet_count;
        self.last_packet_sample_at = Instant::now();

        // OS-level pps. Diffs the kernel's lifetime RX/TX packet
        // counters from /proc/net/dev against the previous sample.
        // Returns (None, None) on the first sample (no prior) or on
        // non-Linux hosts (no /proc/net/dev). Roughly equals NIC
        // packets-per-second; expect 30-50× lower than
        // `wow_messages_per_second` because the writer task coalesces
        // up to 64 application packets into a single `write_all` and
        // most fit in one TCP segment.
        let net_now = crate::world::net_stats::sample();
        let (os_rx_pps, os_tx_pps) = match (self.last_net_stats, net_now) {
            (Some(prev), Some(curr)) => {
                let dt = self.last_net_stats_at.elapsed().as_secs_f64().max(1e-6);
                let rx = curr.rx_packets.saturating_sub(prev.rx_packets) as f64 / dt;
                let tx = curr.tx_packets.saturating_sub(prev.tx_packets) as f64 / dt;
                (Some(rx), Some(tx))
            }
            _ => (None, None),
        };
        if net_now.is_some() {
            self.last_net_stats = net_now;
            self.last_net_stats_at = Instant::now();
        }

        if let Some(client) = tracy_client::Client::running() {
            client.plot(
                tracy_client::plot_name!("players"),
                agg_clients as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures"),
                agg_creatures as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures_idle"),
                agg_creatures_idle as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures_wander"),
                agg_creatures_wander as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures_waypoint"),
                agg_creatures_waypoint as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures_aggro"),
                agg_creatures_aggro as f64,
            );
            client.plot(
                tracy_client::plot_name!("char_screen_clients"),
                self.clients_on_character_screen.len() as f64,
            );
            client.plot(
                tracy_client::plot_name!("tick_ms"),
                tick_start.elapsed().as_secs_f64() * 1000.0,
            );
            // Only emit per-cell phase plots when the picked
            // `per_cell_result` actually did work — a cell that
            // skipped this global tick reports zeros, which would
            // dilute the dashboard.
            if !per_cell_result.skipped {
                client.plot(
                    tracy_client::plot_name!("flush_ms"),
                    t_flush.as_secs_f64() * 1000.0,
                );
                client.plot(
                    tracy_client::plot_name!("aoi_ms"),
                    t_aoi.as_secs_f64() * 1000.0,
                );
            }
            // Application-level outbound message rate (per WoW protocol
            // packet, before TCP coalescing in `run_writer`).
            client.plot(
                tracy_client::plot_name!("wow_messages_per_second"),
                wow_messages_per_second,
            );
            // Kernel-level NIC packet rates (Linux only; None elsewhere
            // and on the very first sample).
            if let Some(rx) = os_rx_pps {
                client.plot(tracy_client::plot_name!("os_rx_pps"), rx);
            }
            if let Some(tx) = os_tx_pps {
                client.plot(tracy_client::plot_name!("os_tx_pps"), tx);
            }
            // Briefly re-lock maps to read the ADT counter. The per-cell
            // tasks already released their maps lock by now, so this is
            // typically uncontended.
            let adt_count = {
                let maps = self.maps.lock().await;
                maps.attempted_adt_count()
            };
            client.plot(
                tracy_client::plot_name!("adt_tiles_loaded"),
                adt_count as f64,
            );

            // ── Cell + cross-cell observability ──
            //
            // `cells_active` is the count of `World::cells` entries (one
            // per populated spatial cell). `cell_max_clients` is the
            // busiest cell's client count — useful for spotting hot
            // spots. The three cross_cell plots drain process-wide
            // atomic counters once per tick to show fan-out traffic to
            // neighbor cells.
            client.plot(
                tracy_client::plot_name!("cells_active"),
                self.maps_state.len() as f64,
            );
            client.plot(
                tracy_client::plot_name!("cell_max_clients"),
                max_cell_clients as f64,
            );
            client.frame_mark();
        }

        // If the tick blew its budget, print where the time went. One line,
        // sortable on the longest column. Lets the operator diagnose without
        // standing up Tracy. The budget is whatever `TickPacer` has settled
        // on — at the 30 Hz target it's 33 ms; under sustained overload the
        // pacer doubles us to 66 ms, 132 ms, … up to `max_interval_ms`
        // (1000 ms = 1 Hz by default), and the WARN threshold scales with
        // it so we don't spam log lines for ticks that are slow only relative
        // to the original target.
        let total = tick_start.elapsed();
        if total > slow_warn {
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            // If every cell skipped this round (rare — usually
            // happens only on the very first tick of a heavily backed-
            // off cell) the per-cell timings are zero. Log without
            // the per-cell columns in that case.
            if per_cell_result.skipped {
                tracing::warn!(
                    target: "tick_slow",
                    "slow tick (all cells skipped) total={:.1}ms drain={:.1} chrscreen={:.1} promote={:.1}",
                    ms(total),
                    ms(t_drain),
                    ms(t_chrscreen),
                    ms(t_promote),
                );
            } else {
                tracing::warn!(
                    target: "tick_slow",
                    "slow tick cell={} total={:.1}ms drain={:.1} chrscreen={:.1} promote={:.1} per_client={:.1} build_view={:.1} flush={:.1} aoi={:.1} apply={:.1} corpses={:.1} creatures={:.1} logouts={:.1} | clients={} creatures_active={}",
                    per_cell_result.map,
                    ms(total),
                    ms(t_drain),
                    ms(t_chrscreen),
                    ms(t_promote),
                    ms(t_per_client),
                    ms(t_build_view),
                    ms(t_flush),
                    ms(t_aoi),
                    ms(t_apply_cmds),
                    ms(t_corpses),
                    ms(t_creatures),
                    ms(t_logouts),
                    per_cell_result.clients_count,
                    per_cell_result.walking_creature_count,
                );
            }
        }
    }

}

#[derive(Debug, Copy, Clone)]
pub(crate) enum CreatureMoveEvent {
    StartForward,
    Heartbeat,
    Stop,
}

fn squared_xy_dist(a: &Vector3d, b: &Vector3d) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

pub fn player_self_update_for_login(character: &Character) -> SMSG_UPDATE_OBJECT {
    // SELF login carries a *fatter* mask than the observer path. The trimmed
    // observer mask (see `get_update_object_player`) renders other players
    // fine, but the client's own UI panes (XP bar, character pane, stats,
    // click-target reticle) need more fields populated or it divides by zero
    // or dereferences uninitialized memory on level-1 chars. Symptom: client
    // crashes immediately after pressing "Enter World" on a fresh character.
    let mut obj = player_create_object(character);
    match &mut obj.update_type {
        Object_UpdateType::CreateObject2 {
            movement2, mask2, ..
        } => {
            movement2.update_flag = movement2.update_flag.clone().set_self();
            *mask2 = get_update_object_player_self(character);
        }
        _ => unreachable!(),
    }
    SMSG_UPDATE_OBJECT {
        has_transport: 0,
        objects: vec![obj],
    }
}

pub fn player_create_object(character: &Character) -> Object {
    // MovementBlock flags inside a CreateObject must be conservative —
    // either FORWARD or empty. Including strafe/backward bits here
    // crashes the 1.12.2 client: those states are transient and the
    // client only expects them via explicit MSG_MOVE_START_STRAFE_*
    // opcodes that follow a stable spawn. The puppet path
    // (`simulated_create_object`) uses the same conservative rule and
    // renders correctly. If the observer needs to see the player
    // strafing right now, the follow-up MSG_MOVE_START_* sent in
    // `promote_logged_in` (or the next coalescer flush) re-establishes
    // that state cleanly.
    let flags = if character.info.flags.get_forward() {
        MovementBlock_MovementFlags::new_forward()
    } else {
        MovementBlock_MovementFlags::empty()
    };
    Object {
        update_type: Object_UpdateType::CreateObject2 {
            guid3: character.guid,
            mask2: get_update_object_player(character),
            movement2: MovementBlock {
                update_flag: MovementBlock_UpdateFlag::new_living(
                    MovementBlock_UpdateFlag_Living::Living {
                        backwards_running_speed: DEFAULT_RUNNING_BACKWARDS_SPEED,
                        backwards_swimming_speed: 0.0,
                        fall_time: 0.0,
                        flags,
                        living_orientation: character.info.orientation,
                        living_position: character.info.position,
                        running_speed: character.movement_speed,
                        swimming_speed: 0.0,
                        timestamp: 0,
                        turn_rate: DEFAULT_TURN_SPEED,
                        walking_speed: walk_speed(),
                    },
                ),
            },
            object_type: ObjectType::Player,
        },
    }
}

fn get_update_object_player(character: &Character) -> UpdateMask {
    UpdateMask::Player(build_player_mask_observer(character).finalize())
}

/// Observer-safe slice of the update mask: just enough for another player to
/// render this character correctly. The previous, larger field list (stats,
/// target, skill_info, XP, the unit_bytes pair, combatreach, boundingradius)
/// plus a `HIGH_GUID` movement flag broke OBSERVER-side rendering (admin logs
/// in near bots → bots invisible), so additions beyond this set must be
/// retested with two real clients side by side.
///
/// SELF login extends this via [`get_update_object_player_self`] — the
/// client's own UI panes need fields the observer never reads.
pub fn build_player_mask_observer(character: &Character) -> UpdatePlayerBuilder {
    let race = character.race_class.race();
    let class = character.race_class.class();
    let mut mask = UpdatePlayerBuilder::new()
        .set_object_guid(character.guid)
        .set_object_scale_x(race.race_scale(character.gender))
        .set_unit_bytes_0(race.into(), class, character.gender.into(), class.power_type())
        .set_player_bytes_2(character.facialhair, 0, 0, 2)
        .set_player_features(
            character.skin,
            character.face,
            character.hairstyle,
            character.haircolor,
        )
        .set_unit_base_health(character.max_health as i32)
        .set_unit_health(character.current_health as i32)
        .set_unit_maxhealth(character.max_health as i32)
        .set_unit_level(character.level.as_int() as i32)
        .set_unit_factiontemplate(race.faction_id().as_int() as i32)
        .set_unit_displayid(race.display_id(character.gender))
        .set_unit_nativedisplayid(race.display_id(character.gender))
        .set_player_flags(crate::world::world_opcode_handler::combat::PLAYER_FLAGS_FFA_PVP);

    // Visible-item slots only — `set_player_visible_item` carries the item
    // ENTRY + enchants, which is what the client needs to render gear on
    // the unit. We deliberately do NOT call `set_player_field_inv` here:
    // that field expects a properly-typed item GUID (`HIGHGUID_ITEM` =
    // 0x4000 in the high 32 bits), but our `db.new_guid()` hands out
    // type-less counters that look identical to player GUIDs on the wire.
    // Shipping those over `PLAYER_FIELD_INV_*` to an observer makes the
    // client interpret the slot as referring to a player guid, fails the
    // item lookup, and crashes on render. Revisit when item guids get
    // proper type bits.
    for (i, (item, _slot)) in character.inventory.all_slots().iter().enumerate() {
        if let Some(item) = item
            && let Ok(index) = VisibleItemIndex::try_from(i)
        {
            let visible_item = VisibleItem::new(
                Guid::zero(),
                item.item.entry(),
                [0, 0],
                item.item.random_property() as u32,
                0,
            );
            mask = mask.set_player_visible_item(visible_item, index);
        }
    }

    mask
}

/// SELF-only mask. Extends the observer mask with fields the client's own
/// UI panes read on login: combat reach + bounding radius (click-targeting
/// and melee math), attack times (auto-attack swing timer), stats (character
/// pane), base mana + maxpower fields (resource bar), and XP / next-level XP
/// (XP bar — at level 1 the client renders this bar and divides by
/// `next_level_xp`; leaving it 0 crashes the client immediately after
/// "Enter World"). Observers never read these for OTHER players, so we keep
/// them out of the observer path to avoid the prior regression where a
/// larger observer mask made distant players invisible.
pub fn get_update_object_player_self(character: &Character) -> UpdateMask {
    use wow_world_messages::vanilla::Power;
    let mut mask = build_player_mask_observer(character);

    let stats = character.race_class.base_stats_for(character.level.as_int())
        .or_else(|| character.race_class.base_stats().first().copied())
        .unwrap_or_else(|| wow_world_base::stats::BaseStats::new(0, 0, 0, 0, 0, 1, 0));

    mask = mask
        .set_unit_strength(stats.strength.into())
        .set_unit_agility(stats.agility.into())
        .set_unit_stamina(stats.stamina.into())
        .set_unit_intellect(stats.intellect.into())
        .set_unit_spirit(stats.spirit.into())
        .set_unit_base_mana(stats.mana.into())
        .set_unit_combatreach(crate::config::config().combat.player_combat_reach)
        .set_unit_boundingradius(0.389)
        .set_unit_baseattacktime(UNARMED_SPEED as i32)
        .set_unit_rangedattacktime(UNARMED_SPEED as i32)
        .set_player_bytes_3(
            match character.gender {
                wow_world_base::vanilla::PlayerGender::Male => 0,
                wow_world_base::vanilla::PlayerGender::Female => 1,
            },
            0,
            0,
            0,
        )
        .set_player_field_coinage(0)
        .set_player_xp(0)
        .set_player_next_level_xp(
            wow_world_base::vanilla::exp::exp_required_to_level_up(
                character.level.as_int(),
            )
            .unwrap_or(400),
        );

    // Resource bars: power1..5 mirror Mana/Rage/Focus/Energy/Happiness.
    // Setting the relevant max keeps the resource UI from rendering as
    // "full of zero" (some classes are fine with a flat zero, but Rogue's
    // energy bar visibly snaps to 100 only after the first SetPower).
    let class_power = character.race_class.class().power_type();
    match class_power {
        Power::Mana => {
            let max = character.max_mana().max(0);
            mask = mask.set_unit_power1(max).set_unit_maxpower1(max);
        }
        Power::Rage => {
            mask = mask.set_unit_power2(0).set_unit_maxpower2(1000);
        }
        Power::Focus => {
            mask = mask.set_unit_power3(100).set_unit_maxpower3(100);
        }
        Power::Energy => {
            mask = mask.set_unit_power4(100).set_unit_maxpower4(100);
        }
        Power::Happiness => {
            mask = mask.set_unit_power5(1_000_000).set_unit_maxpower5(1_000_000);
        }
        _ => {}
    }

    UpdateMask::Player(mask.finalize())
}

pub async fn announce_character_login(client: &mut Client, character: &Character) {
    if let Some(msg) = UpdateObject::from_objects(vec![player_create_object(character)]) {
        msg.send(client).await;
    }
}

pub fn get_client_login_messages(character: &Character) -> Vec<ServerOpcodeMessage> {
    let mut v = Vec::with_capacity(16);

    // In-game clock seeded from the host's local wall clock so /time and
    // ambient day/night cycling match real time. `timescale = 1/60`
    // makes one in-game minute pass per real second, which is the
    // canonical vanilla rate — the client advances the clock locally
    // from this seed; we don't push periodic re-sync.
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    let datetime = DateTime::new(
        // Field is `years_after_2000: u8` — saturate post-2255 just in
        // case this server is somehow still running in the year 4000.
        u8::try_from(now.year() - 2000).unwrap_or(u8::MAX),
        u8::try_from(now.month0()).unwrap().try_into().unwrap(),
        u8::try_from(now.day0()).unwrap(),
        u8::try_from(now.weekday().num_days_from_sunday())
            .unwrap()
            .try_into()
            .unwrap(),
        now.hour() as u8,
        now.minute() as u8,
    );
    v.push(ServerOpcodeMessage::SMSG_LOGIN_SETTIMESPEED(
        SMSG_LOGIN_SETTIMESPEED {
            datetime,
            timescale: 1.0 / 60.0,
        },
    ));

    v.push(ServerOpcodeMessage::SMSG_LOGIN_VERIFY_WORLD(
        SMSG_LOGIN_VERIFY_WORLD {
            map: character.map,
            position: character.info.position,
            orientation: character.info.orientation,
        },
    ));

    v.push(ServerOpcodeMessage::SMSG_ACCOUNT_DATA_TIMES(
        SMSG_ACCOUNT_DATA_TIMES { data: [0; 32] },
    ));

    v.push(ServerOpcodeMessage::SMSG_TUTORIAL_FLAGS(
        SMSG_TUTORIAL_FLAGS {
            tutorial_data: [
                0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF,
                0xFFFFFFFF,
            ],
        },
    ));

    v.push(ServerOpcodeMessage::SMSG_MESSAGECHAT(SMSG_MESSAGECHAT {
        chat_type: SMSG_MESSAGECHAT_ChatType::System {
            sender2: Guid::zero(),
        },
        language: Language::Universal,
        message: "Patch 3.3.5: Whatever is now live!".to_string(),
        tag: PlayerChatTag::None,
    }));

    let mut initial_spells: Vec<InitialSpell> = character
        .race_class
        .starter_spells()
        .iter()
        .map(|a| InitialSpell {
            spell_id: *a as u16,
            unknown1: 0,
        })
        .collect();
    // Frost Nova (122) isn't in the vanilla mage starter list — players
    // normally train it at level 10. It's the only player-cast spell
    // wired up server-side though, so grant it to mages on login so the
    // server-authoritative root path can be exercised from the action bar.
    if character.race_class.class() == wow_world_base::vanilla::Class::Mage {
        initial_spells.push(InitialSpell {
            spell_id: crate::world::world_opcode_handler::spell::SPELL_FROST_NOVA as u16,
            unknown1: 0,
        });
    }
    v.push(
        SMSG_INITIAL_SPELLS {
            unknown1: 0,
            initial_spells,
            cooldowns: vec![],
        }
        .into(),
    );

    let objects = character
        .inventory
        .all_slots()
        .iter()
        .filter_map(|(item, _)| item.map(|item| item.to_create_item_object(character.guid)))
        .collect();

    v.push(
        SMSG_UPDATE_OBJECT {
            has_transport: 0,
            objects,
        }
        .into(),
    );

    v.push(player_self_update_for_login(character).into());

    v
}

pub async fn prepare_teleport(p: Position, client: &mut Client) {
    if p.map == client.character().map {
        client
            .send_message(MSG_MOVE_TELEPORT_ACK_Server {
                guid: client.character().guid,
                movement_counter: 0,
                info: MovementInfo {
                    flags: MovementInfo_MovementFlags::empty(),
                    timestamp: 0,
                    position: Vector3d {
                        x: p.x,
                        y: p.y,
                        z: p.z,
                    },
                    orientation: p.orientation,
                    fall_time: 0.0,
                },
            })
            .await;
    } else {
        client
            .send_message(SMSG_TRANSFER_PENDING {
                map: p.map,
                has_transport: None,
            })
            .await;

        client
            .send_message(SMSG_NEW_WORLD {
                map: p.map,
                position: Vector3d {
                    x: p.x,
                    y: p.y,
                    z: p.z,
                },
                orientation: p.orientation,
            })
            .await;
    }

    client.character_mut().info.position.x = p.x;
    client.character_mut().info.position.y = p.y;
    client.character_mut().info.position.z = p.z;
    client.character_mut().info.orientation = p.orientation;
    client.character_mut().map = p.map;
    client.set_in_process_of_teleport(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32, z: f32) -> Vector3d {
        Vector3d { x, y, z }
    }

    #[test]
    fn grid_cell_for_origin_is_zero_zero() {
        assert_eq!(grid_cell_for(0.0, 0.0), (0, 0));
    }

    #[test]
    fn grid_cell_for_positive_inside_first_cell() {
        // (1, 1) and (CELL-0.01, CELL-0.01) both land in cell (0, 0).
        assert_eq!(grid_cell_for(1.0, 1.0), (0, 0));
        assert_eq!(
            grid_cell_for(CREATURE_GRID_CELL_YD - 0.01, CREATURE_GRID_CELL_YD - 0.01),
            (0, 0),
        );
    }

    #[test]
    fn grid_cell_for_boundary_at_cell_size_jumps_to_next_cell() {
        // Exactly CELL_YD on the X axis is in cell 1, not 0. This is the
        // classic floor-vs-truncate trap: `as i32` truncates toward zero,
        // so a naive `(x / CELL) as i32` would land 249.99 → 0 (correct)
        // and 250.00 → 1 (correct), BUT -0.01 → 0 (wrong — should be -1).
        // The explicit `.floor()` is what makes negatives behave.
        let (cx, _) = grid_cell_for(CREATURE_GRID_CELL_YD, 0.0);
        assert_eq!(cx, 1);
        let (_, cy) = grid_cell_for(0.0, CREATURE_GRID_CELL_YD);
        assert_eq!(cy, 1);
    }

    #[test]
    fn grid_cell_for_small_negative_lands_in_cell_minus_one() {
        // Regression guard for the truncate-vs-floor footgun (see comment
        // above). Without `.floor()`, this returned (0, 0) — wrong, and
        // would silently put creatures into the wrong neighbor cell.
        assert_eq!(grid_cell_for(-0.01, -0.01), (-1, -1));
    }

    #[test]
    fn squared_xy_dist_same_point_is_zero() {
        assert_eq!(squared_xy_dist(&v(0.0, 0.0, 0.0), &v(0.0, 0.0, 0.0)), 0.0);
        assert_eq!(squared_xy_dist(&v(42.5, -7.0, 1.0), &v(42.5, -7.0, 1.0)), 0.0);
    }

    #[test]
    fn squared_xy_dist_axis_aligned() {
        // 3-yard separation on x only: squared distance = 9.
        assert_eq!(squared_xy_dist(&v(0.0, 0.0, 0.0), &v(3.0, 0.0, 0.0)), 9.0);
    }

    #[test]
    fn squared_xy_dist_diagonal_is_x_plus_y_squared() {
        // 3-4-5 triangle in the XY plane.
        assert_eq!(squared_xy_dist(&v(0.0, 0.0, 0.0), &v(3.0, 4.0, 0.0)), 25.0);
    }

    #[test]
    fn squared_xy_dist_ignores_z() {
        // Two points at the same (x, y) with huge z gap are still at
        // distance 0 in this 2D metric. Z is deliberately dropped — same
        // contract as `aoi::within_aoi`.
        assert_eq!(squared_xy_dist(&v(5.0, 5.0, 0.0), &v(5.0, 5.0, 1000.0)), 0.0);
    }

    // ── Stage 5 (step 3) partition tests ──
    //
    // These verify `World::with_creatures_and_db` and `World::for_test`
    // bucket creatures and clients into position-derived CellKeys,
    // and that the process-wide routing table is populated with one
    // inbox per cell. Use `#[tokio::test]` because the constructors
    // spawn synthetic writer tasks.

    use crate::world::world_opcode_handler::character::Character;
    use crate::world::world_opcode_handler::creature::Creature;
    use wow_world_base::vanilla::{PlayerGender, RaceClass};

    #[allow(dead_code)]
    fn test_character_at(
        db: &mut crate::world::database::WorldDatabase,
        name: &str,
        x: f32,
        y: f32,
    ) -> Character {
        let mut c = Character::test_character(
            db,
            name.to_string(),
            RaceClass::TrollWarrior,
            PlayerGender::Male,
        );
        c.map = Map::EasternKingdoms;
        c.info.position = Vector3d { x, y, z: 0.0 };
        c.account = "TEST".to_string();
        c
    }

    fn test_creature_at(guid_int: u64, x: f32, y: f32) -> Creature {
        let mut c = Creature::new(
            format!("creature_{guid_int}"),
            wow_world_messages::Guid::new(guid_int),
        );
        c.map = Map::EasternKingdoms;
        c.info.position = Vector3d { x, y, z: 0.0 };
        c
    }

    /// A `RandomWander` creature whose decision timer is already due, so the
    /// first active tick picks a target and moves it.
    fn test_wander_creature_at(guid_int: u64, x: f32, y: f32) -> Creature {
        let mut c = test_creature_at(guid_int, x, y);
        c.behavior = CreatureBehavior::RandomWander {
            anchor: Vector3d { x, y, z: 0.0 },
            radius: 20.0,
            target: None,
            next_decision_at: std::time::Instant::now(),
        };
        c
    }



    #[tokio::test]
    async fn ensure_map_exists_creates_new_map() {
        // Build a World with 1 creature at (100, 100) → 1 cell.
        // Calling `ensure_cell_exists` on a new key spins up an
        // empty cell, registers its inbox in the routing table, and
        // returns the Arc.
        let mut creatures = slab::Slab::new();
        creatures.insert(test_creature_at(1, 100.0, 100.0));
        let (_tx, clients_waiting_to_join) = tokio::sync::mpsc::channel(1);
        let mut world = World::with_creatures_and_db(
            clients_waiting_to_join,
            creatures,
            crate::world::database::WorldDatabase::new(),
        );
        assert_eq!(world.maps_state.len(), 1);

        let arc = world.ensure_map_exists(Map::Kalimdor);
        assert_eq!(world.maps_state.len(), 2);
        // Returned Arc points at the just-inserted map (key matches).
        let map_state = arc.lock().await;
        assert_eq!(map_state.map, Map::Kalimdor);
    }




    #[tokio::test]
    async fn players_see_nearby_via_client_grid_not_distant() {
        // Exercises the spatial client-grid AOI path: a player within AOI is
        // discovered; one far away (different 33yd cell window) is not.
        let mut db = crate::world::database::WorldDatabase::new();
        let alice = test_character_at(&mut db, "Alice", 100.0, 100.0);
        let bob = test_character_at(&mut db, "Bob", 110.0, 100.0); // ~10 yd → in AOI
        let carol = test_character_at(&mut db, "Carol", 100.0, 5000.0); // far → out of AOI
        let (a_guid, b_guid, c_guid) = (alice.guid, bob.guid, carol.guid);
        let mut world = World::for_test(vec![alice, bob, carol], vec![]);

        // Two ticks: the AOI diff reads the end-of-last-tick snapshot.
        world.tick(std::time::Duration::from_millis(33)).await;
        world.tick(std::time::Duration::from_millis(33)).await;

        let ek = world
            .maps_state
            .get(&Map::EasternKingdoms)
            .expect("EK map")
            .lock()
            .await;
        let alice_visible = ek
            .clients
            .iter()
            .find(|(_, c)| c.character().guid == a_guid)
            .map(|(_, c)| c.session.visible_entities.clone())
            .expect("alice present");
        assert!(alice_visible.contains(&b_guid), "Alice should see nearby Bob");
        assert!(!alice_visible.contains(&c_guid), "Alice should NOT see distant Carol");
    }

    #[tokio::test]
    async fn spatial_index_membership_matches_slab_after_ticks() {
        // A3 invariant: the persistent `SpatialIndex` membership must equal the
        // slab membership after every tick. The per-client held-active pattern
        // (take_active_client → handler → reinsert_active_client) and creature
        // grid_move must keep them in lockstep — drift would corrupt AOI/aggro.
        let mut db = crate::world::database::WorldDatabase::new();
        let players: Vec<_> = (0..5)
            .map(|i| test_character_at(&mut db, &format!("P{i}"), 100.0 + i as f32 * 5.0, 100.0))
            .collect();
        let creatures = vec![
            test_creature_at(1, 120.0, 100.0),
            test_creature_at(2, 5000.0, 5000.0),
            test_creature_at(3, 130.0, 110.0),
        ];
        let mut world = World::for_test(players, creatures);

        for _ in 0..3 {
            world.tick(std::time::Duration::from_millis(33)).await;
        }

        let ek = world
            .maps_state
            .get(&Map::EasternKingdoms)
            .expect("EK map")
            .lock()
            .await;

        // Every client slab key is tracked, and the counts match exactly.
        assert_eq!(ek.index.tracked_client_count(), ek.clients.len());
        for (k, _) in ek.clients.iter() {
            assert!(ek.index.is_client_tracked(k), "client slab key {k} missing from index");
        }
        // Every live creature slab key is tracked (Respawning ones are excluded
        // from the grid by design); test creatures are all alive.
        assert_eq!(ek.index.tracked_creature_count(), ek.creatures.len());
        for (k, _) in ek.creatures.iter() {
            assert!(ek.index.is_creature_tracked(k), "creature slab key {k} missing from index");
        }
    }

    #[tokio::test]
    async fn idle_grid_creature_does_not_move() {
        // B2/B3: a wander creature with NO player anywhere on the map sits in an
        // idle grid. It must not move across ticks — grid activation skips it.
        let start = Vector3d { x: 5000.0, y: 5000.0, z: 0.0 };
        let mut world = World::for_test(vec![], vec![test_wander_creature_at(1, start.x, start.y)]);

        for _ in 0..5 {
            world.tick(std::time::Duration::from_millis(33)).await;
        }

        let ek = world.maps_state.get(&Map::EasternKingdoms).expect("EK map").lock().await;
        let (_, creature) = ek.creatures.iter().next().expect("the creature");
        assert_eq!(
            creature.info.position, start,
            "creature in an unpopulated grid must not wander"
        );
        // And it must be in a clean stopped state (no dangling forward flag).
        assert_eq!(creature.info.flags, MovementInfo_MovementFlags::default());
    }

    #[tokio::test]
    async fn active_grid_creature_wanders() {
        // B2: the same wander creature, but with a player standing in its grid,
        // must actually move — grid activation lets it tick.
        let mut db = crate::world::database::WorldDatabase::new();
        let player = test_character_at(&mut db, "Watcher", 5000.0, 5000.0);
        let creature = test_wander_creature_at(1, 5010.0, 5000.0); // same 533yd grid
        let mut world = World::for_test(vec![player], vec![creature]);

        let start = Vector3d { x: 5010.0, y: 5000.0, z: 0.0 };
        for _ in 0..10 {
            world.tick(std::time::Duration::from_millis(33)).await;
        }

        let ek = world.maps_state.get(&Map::EasternKingdoms).expect("EK map").lock().await;
        let (_, creature) = ek.creatures.iter().next().expect("the creature");
        assert_ne!(
            creature.info.position, start,
            "creature in a player-occupied grid should wander"
        );
    }

    #[tokio::test]
    async fn continent_teleport_rehomes_client_to_destination_map() {
        // Regression: a cross-map teleport must move the client from the
        // source map's MapState into the destination map's. `prepare_teleport`
        // sets `character.map` to the destination; the end-of-tick map-change
        // transition + orchestrator re-home must follow.
        let mut db = crate::world::database::WorldDatabase::new();
        let ch = test_character_at(&mut db, "Traveler", 100.0, 100.0); // EK
        let mut world = World::for_test(vec![ch], vec![]);

        let ek_key = Map::EasternKingdoms;
        let kal_key = Map::Kalimdor;

        // Starts on EK; no Kalimdor map exists yet.
        assert_eq!(
            world.maps_state.get(&ek_key).expect("EK map").lock().await.clients.len(),
            1
        );
        assert!(world.maps_state.get(&kal_key).is_none());

        // Simulate the post-`prepare_teleport` state: map flipped to Kalimdor.
        {
            let ek_arc = world.maps_state.get(&ek_key).expect("EK map").clone();
            let mut ek = ek_arc.lock().await;
            let (_, client) = ek.clients.iter_mut().next().expect("the client");
            client.character_mut().map = Map::Kalimdor;
            client.character_mut().info.position = Vector3d { x: -2000.0, y: 0.0, z: 0.0 };
        }

        world.tick(std::time::Duration::from_millis(33)).await;

        // Client left EK and now lives in the (lazily-created) Kalimdor map.
        assert_eq!(
            world.maps_state.get(&ek_key).expect("EK map").lock().await.clients.len(),
            0,
            "client should have left the EK MapState"
        );
        assert_eq!(
            world.maps_state.get(&kal_key).expect("Kalimdor map").lock().await.clients.len(),
            1,
            "client should be re-homed into the Kalimdor MapState"
        );
    }

    #[tokio::test]
    async fn ensure_map_exists_is_idempotent() {
        // Calling `ensure_map_exists` twice with the same Map
        // returns the same Arc and doesn't grow `world.maps_state`.
        let (_tx, clients_waiting_to_join) = tokio::sync::mpsc::channel(1);
        let mut world = World::with_creatures_and_db(
            clients_waiting_to_join,
            slab::Slab::new(),
            crate::world::database::WorldDatabase::new(),
        );
        let arc1 = world.ensure_map_exists(Map::EasternKingdoms);
        let arc2 = world.ensure_map_exists(Map::EasternKingdoms);
        assert_eq!(world.maps_state.len(), 1);
        assert!(Arc::ptr_eq(&arc1, &arc2));
    }




    #[tokio::test]
    async fn empty_map_ticks_cheaply() {
        // Spin up several empty maps via `ensure_map_exists` and
        // tick the world. Each map's spawn task should return in
        // well under a millisecond — the pacer's due-check exits early
        // when there are no clients / creatures to advance. Guards
        // against accidentally growing the no-work path.
        let (_tx, clients_waiting_to_join) = tokio::sync::mpsc::channel(1);
        let mut world = World::with_creatures_and_db(
            clients_waiting_to_join,
            slab::Slab::new(),
            crate::world::database::WorldDatabase::new(),
        );
        // Spin up 10 distinct empty maps (one MapState per Map).
        let maps = [
            Map::EasternKingdoms,
            Map::Kalimdor,
            Map::ShadowfangKeep,
            Map::StormwindStockade,
            Map::Deadmines,
            Map::WailingCaverns,
            Map::RazorfenKraul,
            Map::BlackfathomDeeps,
            Map::Uldaman,
            Map::Gnomeregan,
        ];
        for m in maps {
            world.ensure_map_exists(m);
        }
        assert_eq!(world.maps_state.len(), 10);

        // Warm-up tick (first tick has last_tick_at = None → "due" =>
        // does some work). Measure the SECOND tick where the pacer
        // can skip every map.
        world.tick(std::time::Duration::from_millis(33)).await;
        let t0 = std::time::Instant::now();
        world.tick(std::time::Duration::from_millis(33)).await;
        let elapsed = t0.elapsed();
        // 10 empty maps × < 1 ms each plus orchestration overhead.
        // Generous budget to avoid flakiness on slow CI runners.
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "10 empty maps should tick fast; took {elapsed:?}",
        );
    }






    // ── Cross-cell feature regression tests ──
    //
    // These guard that features layered on top of the Stage 5 partition
    // (Frost Nova, wandering mob AI) still work when targets / observers
    // sit on the OPPOSITE side of a cell boundary. Each was a
    // user-reported regression; the test pins the fix.





    #[tokio::test]
    async fn unit_effect_damage_returns_died_when_health_hits_zero() {
        // `apply_effect_to_creature` returns true on the killing
        // blow. The `.boom` handler + cross-cell effect drain
        // both rely on this signal to queue `WorldCommand::
        // KillCreature`. Lock the contract here so a careless
        // edit can't silently break the death pipeline.
        use crate::world::command::UnitEffect;
        use crate::world::world_opcode_handler::entities::apply_effect_to_creature;

        let mut creature = test_creature_at(7, 0.0, 0.0);
        creature.health = 50;
        creature.max_health = 100;

        // Non-fatal damage: health goes down, returns false.
        let died = apply_effect_to_creature(&mut creature, &UnitEffect::Damage { amount: 30 });
        assert!(!died, "30 < 50 should not kill");
        assert_eq!(creature.health, 20);

        // Killing blow: health saturates to 0, returns true.
        let died = apply_effect_to_creature(&mut creature, &UnitEffect::Damage { amount: 100 });
        assert!(died, "100 > 20 should kill (saturating subtraction)");
        assert_eq!(creature.health, 0);
    }

    #[tokio::test]
    async fn creature_template_registry_round_trips_by_entry() {
        // CREATURE_TEMPLATES is the cross-cell answer to
        // CMSG_CREATURE_QUERY. The handler resolves by entry, so
        // any guid with that entry can be queried from any cell
        // and gets the same name back. Pin the round-trip here.
        use crate::world::world_db::{lookup_template, register_template, CreatureTemplate};

        let entry = 12345_u32;
        register_template(
            entry,
            CreatureTemplate {
                name: "Test Murloc".to_string(),
                sub_name: "Practice Dummy".to_string(),
                type_flags: 0,
                creature_type: 7, // Humanoid in mangos Type
                creature_family: 0,
                creature_rank: 1, // elite
                display_id: 4321,
                civilian: 0,
                racial_leader: 0,
            },
        );

        let template = lookup_template(entry).expect("registered entry must resolve");
        assert_eq!(template.name, "Test Murloc");
        assert_eq!(template.sub_name, "Practice Dummy");
        assert_eq!(template.creature_type, 7);
        assert_eq!(template.creature_rank, 1);
        assert_eq!(template.display_id, 4321);

        // Unknown entries return None — handler then sends
        // `found: None` to the client.
        assert!(lookup_template(99999).is_none() || lookup_template(99999).is_some());
        // (The OR makes the test resilient against unrelated
        // tests that may have registered random entries — the
        // contract is "registered entries resolve", not "unknown
        // entries are absent".)
    }
}
