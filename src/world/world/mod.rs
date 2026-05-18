use crate::world::aoi;
use crate::world::character_screen_handler::handle_character_screen_opcodes;
use crate::world::database::WorldDatabase;
use crate::world::region::RegionKey;
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

/// Per-region game state. In Stage 1 of the per-region refactor there's
/// exactly one `RegionState` per `World` — covering every player on every
/// map — so behavior is unchanged. Subsequent stages partition this state
/// across many regions, each running its own tick loop independently.
///
/// Field accesses from inside `impl World` are spelled `self.region_mut().X`.
/// Methods that act purely on per-region state will eventually move to
/// `impl RegionState`; for now they stay on `World` and pierce through
/// `self.region` to keep this commit purely mechanical (no method moves,
/// no behavior change).
#[derive(Debug)]
pub struct RegionState {
    /// Identity of this region. Currently every `World` holds exactly
    /// one `RegionState` keyed by [`World::WORLD_KEY`] (a sentinel that
    /// names "the single-region world"); partition into actual
    /// position-derived keys is a follow-up Stage 3 task.
    pub(crate) key: RegionKey,
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

    /// Spatial index for AOI queries — maps `(Map, cell_x, cell_y)` to the
    /// slab keys of creatures currently in that 250-yd cell. In Stage 1
    /// the key still carries `Map` because the single region spans all
    /// maps; once regions partition by map+spatial-cell the key drops
    /// `Map` (each region's grid is already implicitly map-local).
    pub(crate) creature_cells: ahash::AHashMap<(Map, i32, i32), Vec<usize>>,
    pub(crate) creature_cell_of: ahash::AHashMap<usize, (Map, i32, i32)>,

    /// Start of this region's previous tick. Used to compute wall-clock
    /// `dt` for time-dependent state like `auto_attack_timer`. Per-region
    /// because each region will eventually pace independently.
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
    pub(crate) scratch_client_aabb: ahash::AHashMap<Map, (f32, f32, f32, f32)>,
    pub(crate) scratch_walk_events: Vec<(usize, Vector3d, Map, CreatureMoveEvent)>,
    pub(crate) scratch_to_park: Vec<(Instant, usize)>,
    pub(crate) scratch_parked_set: ahash::AHashSet<usize>,
    pub(crate) scratch_expired_roots: Vec<(Guid, Map, Vector3d, MovementInfo)>,

    /// Per-tick `Sync`-safe view of `clients`, rebuilt at the top of the
    /// broadcast phase from each `Client::broadcast_target()`.
    pub(crate) broadcast_view: Vec<crate::world::aoi::BroadcastTarget>,

    /// Per-region adaptive pacer. Today the global `run_world` loop
    /// is the only thing that actually *sleeps* on a pacer's
    /// `current_interval`; this per-region pacer observes each tick's
    /// per-region duration and publishes its state to the
    /// `region::PACER_STATE` snapshot so `.regions` can show how a
    /// region would back off independently once Stage 3 polish wires
    /// long-lived per-region task loops.
    pub(crate) pacer: crate::world::TickPacer,
}

#[derive(Debug)]
pub struct World {
    /// All per-region state, keyed by [`RegionKey`]. Stage 3 wraps each
    /// region in an `Arc<Mutex<>>` so it can move into a `tokio::spawn`ed
    /// task — the global tick spawns one task per region and joins them
    /// all at the end of the per-region phase, giving true tokio-worker
    /// parallelism across regions.
    pub regions: ahash::AHashMap<RegionKey, Arc<Mutex<RegionState>>>,

    // ── Global state (auth + connection lifecycle + cross-cutting metrics)
    // Stays on `World` indefinitely — these are NOT region-scoped.
    pub(crate) clients_on_character_screen: Vec<CharacterScreenClient>,
    pub(crate) clients_waiting_to_join: Receiver<CharacterScreenClient>,
    /// Wrapped in `Arc<Mutex<>>` so the per-region tokio tasks can each
    /// borrow it; `ground_height` and ADT-load both mutate so we can't
    /// just `Arc<>` and need exterior synchronization. Contention is
    /// low — only `apply_commands` and `tick_creature_ai` lock.
    pub(crate) maps: Arc<Mutex<PathfindingMaps>>,
    /// Wrapped likewise so promote (global) and per_client_loop /
    /// stale-cleanup (per-region) can share access. The lock is held
    /// only while the opcode handler runs for one client at a time —
    /// regions still tick the rest of their work in parallel.
    pub(crate) db: Arc<Mutex<WorldDatabase>>,
    pub(crate) last_packet_sample: u64,
    pub(crate) last_packet_sample_at: Instant,
    pub(crate) last_net_stats: Option<crate::world::net_stats::NetStats>,
    pub(crate) last_net_stats_at: Instant,
}

impl World {
    /// Sentinel region key for the single-region build. Once Stage 3
    /// partitions clients/creatures by position this constant goes
    /// away — every region will own a true geometric `RegionKey`.
    pub const WORLD_KEY: RegionKey = RegionKey {
        map: wow_world_base::vanilla::Map::EasternKingdoms,
        rx: i32::MIN,
        ry: i32::MIN,
    };

    /// `Arc<Mutex<RegionState>>` for the (currently sole) region. Panics
    /// if the map is empty.
    #[inline]
    pub fn primary_region(&self) -> Arc<Mutex<RegionState>> {
        self.regions
            .get(&Self::WORLD_KEY)
            .expect("primary region must exist")
            .clone()
    }
}

#[derive(Debug)]
pub(crate) struct PendingMovement {
    pub msg: ServerOpcodeMessage,
    pub anchor: Vector3d,
    pub map: Map,
}

/// Cell size (yards) for the creature spatial grid. Picked larger than the
/// configured AOI radius so a 3×3 cell scan around an observer covers every
/// candidate within AOI without needing a wider scan. Used by both the
/// grid build and the AOI-transition lookup — keep in sync.
pub const CREATURE_GRID_CELL_YD: f32 = 250.0;

/// Compute the spatial-grid cell key for a creature at `(map, x, y)`. Z is
/// deliberately ignored — AOI is horizontal-only, same as `within_aoi`.
#[inline]
fn grid_cell_for(map: Map, x: f32, y: f32) -> (Map, i32, i32) {
    let cx = (x / CREATURE_GRID_CELL_YD).floor() as i32;
    let cy = (y / CREATURE_GRID_CELL_YD).floor() as i32;
    (map, cx, cy)
}

/// Bundle of per-region tick outputs returned by the `tokio::spawn`ed
/// region task to the orchestrator. Used for the slow-tick log,
/// post-spawn Tracy plots, and re-admitting logged-out clients into
/// `clients_on_character_screen`.
#[derive(Debug)]
pub struct PerRegionTickResult {
    pub region_key: RegionKey,
    /// True if this region's pacer told it to skip this global tick:
    /// the global orchestrator is running at 30 Hz, but this region's
    /// pacer has backed off (e.g. to 100 ms = 10 Hz) so it sat out
    /// the past `(target / current_interval) - 1` global ticks. The
    /// orchestrator suppresses Tracy plots and the slow-tick log for
    /// skipped regions so the dashboard only shows ticks that did
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
    /// Sum of every per-region phase timing. Plotted as `region_tick_ms`
    /// so the dashboard can compare per-region tick cost; the global
    /// `tick_ms` plot covers the orchestrator (global phases + spawn
    /// orchestration + this region's work).
    pub t_region_total: Duration,
    pub departed: Vec<CharacterScreenClient>,
    pub clients_count: usize,
    pub creatures_count: usize,
    pub creature_idle_count: usize,
    pub creature_wander_count: usize,
    pub creature_waypoint_count: usize,
    pub creature_aggro_count: usize,
    pub walking_creature_count: usize,
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

impl RegionState {
    /// Insert a client into the slab and keep `client_by_guid` in sync.
    /// Always use this rather than `self.clients.insert(...)` directly
    /// so the reverse index stays authoritative.
    pub(crate) fn insert_client(&mut self, c: Client) -> usize {
        let guid = c.character().guid;
        let key = self.clients.insert(c);
        self.client_by_guid.insert(guid, key);
        key
    }

    /// Remove a client from the slab and drop the matching
    /// `client_by_guid` entry. Pairs with [`Self::insert_client`].
    pub(crate) fn remove_client(&mut self, key: usize) -> Client {
        let c = self.clients.remove(key);
        self.client_by_guid.remove(&c.character().guid);
        c
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
        if self.creature_cell_of.contains_key(&key) {
            return; // already in the grid
        }
        let cell = grid_cell_for(c.map, c.info.position.x, c.info.position.y);
        self.creature_cells.entry(cell).or_default().push(key);
        self.creature_cell_of.insert(key, cell);
    }

    /// Remove `key` from the grid if present. Used on Corpse → Respawning
    /// transitions and on creature destruction. Cheap when absent.
    pub(crate) fn grid_remove(&mut self, key: usize) {
        let Some(cell) = self.creature_cell_of.remove(&key) else {
            return;
        };
        if let Some(bucket) = self.creature_cells.get_mut(&cell) {
            if let Some(pos) = bucket.iter().position(|&k| k == key) {
                bucket.swap_remove(pos);
            }
            if bucket.is_empty() {
                self.creature_cells.remove(&cell);
            }
        }
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
            self.grid_remove(key);
            return;
        }
        let new_cell = grid_cell_for(c.map, c.info.position.x, c.info.position.y);
        let prev = self.creature_cell_of.get(&key).copied();
        if prev == Some(new_cell) {
            return; // same cell — no work
        }
        if let Some(old) = prev
            && let Some(bucket) = self.creature_cells.get_mut(&old)
        {
            if let Some(pos) = bucket.iter().position(|&k| k == key) {
                bucket.swap_remove(pos);
            }
            if bucket.is_empty() {
                self.creature_cells.remove(&old);
            }
        }
        self.creature_cells.entry(new_cell).or_default().push(key);
        self.creature_cell_of.insert(key, new_cell);
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

    #[tracing::instrument(level = "info", skip_all, name = "tick_creature_ai")]
    pub(crate) async fn tick_creature_ai(&mut self, maps: &mut PathfindingMaps) {
        let creature_cfg = &crate::config::config().creature;
        let re_path_threshold = creature_cfg.re_path_threshold;
        let stand_off = creature_cfg.stand_off;
        let max_follow_range = creature_cfg.max_follow_range;

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

        let clients = &self.clients;
        let targets: Vec<(usize, Option<usize>)> = self
            .aggro_creature_keys
            .par_iter()
            .map(|&creature_key| {
                let creature = &self.creatures[creature_key];
                if creature.is_rooted() {
                    return (creature_key, None);
                }
                let from = creature.info.position;
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

        while let Some((&t, _)) = self.creature_wake_at.iter().next() {
            if t > now {
                break;
            }
            let keys = self.creature_wake_at.remove(&t).unwrap_or_default();
            for k in keys {
                if self.creatures.contains(k) {
                    self.walking_creature_keys.push(k);
                }
            }
        }

        self.scratch_client_aabb.clear();
        let aoi_r = crate::config::config().network.aoi_radius_yards;
        for (_, cl) in self.clients.iter() {
            let p = cl.character().info.position;
            let map = cl.character().map;
            let entry = self
                .scratch_client_aabb
                .entry(map)
                .or_insert((f32::MAX, f32::MAX, f32::MIN, f32::MIN));
            entry.0 = entry.0.min(p.x - aoi_r);
            entry.1 = entry.1.min(p.y - aoi_r);
            entry.2 = entry.2.max(p.x + aoi_r);
            entry.3 = entry.3.max(p.y + aoi_r);
        }

        let mut events = std::mem::take(&mut self.scratch_walk_events);
        events.clear();
        let mut to_park = std::mem::take(&mut self.scratch_to_park);
        to_park.clear();
        let client_aabb = std::mem::take(&mut self.scratch_client_aabb);

        let walking_keys = std::mem::take(&mut self.walking_creature_keys);
        for &key in &walking_keys {
            let c = &mut self.creatures[key];
            let Some(aabb) = client_aabb.get(&c.map) else {
                continue;
            };
            let p = &c.info.position;
            if p.x < aabb.0 || p.x > aabb.2 || p.y < aabb.1 || p.y > aabb.3 {
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
                events.push((key, c.info.position, map, CreatureMoveEvent::StartForward));
            }

            let dx = target.x - c.info.position.x;
            let dy = target.y - c.info.position.y;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist <= step || dist <= arrival_threshold {
                c.info.position = target;
                c.info.flags = MovementInfo_MovementFlags::default();
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
        self.scratch_client_aabb = client_aabb;

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
    pub async fn tick_aoi_transitions(&mut self) -> AoiTickStats {
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
        // creature_cells map, client_by_guid, clients slab). All
        // writes are to the moved-in `visible` / `transition` /
        // returned vecs — no cross-observer state. Rayon spreads the
        // observers across its thread pool.
        struct DiffResult {
            key: usize,
            visible_entities: ahash::AHashSet<Guid>,
            aoi_transition_at: ahash::AHashMap<Guid, Instant>,
            departed: Vec<Guid>,
            entered: Vec<Guid>,
            entered_objects: Vec<Object>,
            fast_path: bool,
            suppressed: usize,
        }
        let results: Vec<DiffResult> = {
            let _s = tracing::info_span!("aoi_diff_parallel").entered();
            let broadcast_view = &self.broadcast_view;
            let creatures = &self.creatures;
            let creature_cells = &self.creature_cells;
            let creature_by_guid = &self.creature_by_guid;
            let client_by_guid = &self.client_by_guid;
            let clients = &self.clients;
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

                    // Build new_visible: scan broadcast_view + creatures
                    // via the 3×3 cell window.
                    let mut new_visible: ahash::AHashSet<Guid> =
                        ahash::AHashSet::with_capacity(visible.len());
                    for t in broadcast_view {
                        if t.guid != observer_guid
                            && t.map == observer_map
                            && aoi::within_aoi_sq(&t.position, &observer_pos, aoi_r_sq)
                        {
                            new_visible.insert(t.guid);
                        }
                    }
                    let cx = (observer_pos.x / CREATURE_GRID_CELL_YD).floor() as i32;
                    let cy = (observer_pos.y / CREATURE_GRID_CELL_YD).floor() as i32;
                    for dx in -1..=1 {
                        for dy in -1..=1 {
                            let Some(keys) =
                                creature_cells.get(&(observer_map, cx + dx, cy + dy))
                            else {
                                continue;
                            };
                            for &ck in keys {
                                let cr = &creatures[ck];
                                if aoi::within_aoi_sq(
                                    &observer_pos,
                                    &cr.info.position,
                                    aoi_r_sq,
                                ) {
                                    new_visible.insert(cr.guid);
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
                            entered_objects: Vec::new(),
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

                    // Build entered_objects (CreateObject2 for each
                    // newcomer guid). Reads from creatures + clients
                    // slabs via the reverse-index maps. Reading another
                    // observer's `c.character()` here is fine — only
                    // their Character is touched, never their session.
                    let mut entered_objects = Vec::with_capacity(entered.len());
                    for g in &entered {
                        if let Some(&ck) = creature_by_guid.get(g)
                            && let Some(cr) = creatures.get(ck)
                        {
                            entered_objects.push(cr.to_create_object());
                            continue;
                        }
                        if let Some(&pk) = client_by_guid.get(g)
                            && let Some(c) = clients.get(pk)
                        {
                            entered_objects.push(player_create_object(c.character()));
                        }
                    }

                    DiffResult {
                        key,
                        visible_entities: new_visible,
                        aoi_transition_at: transition,
                        departed,
                        entered,
                        entered_objects,
                        fast_path: false,
                        suppressed: suppressed_count,
                    }
                })
                .collect()
        };

        // ── PHASE 2 (sequential): restore per-observer state, send
        // packets. Async sends keep this single-threaded.
        for result in results {
            stats.fast_path += if result.fast_path { 1 } else { 0 };
            stats.suppressed += result.suppressed;
            stats.departed += result.departed.len();
            stats.entered += result.entered.len();

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
            UpdateObject::send_chunked(result.entered_objects, obs).await;
        }
        }
        .instrument(tracing::info_span!("tick_aoi_transitions"))
        .await;
        stats
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
        self.creature_cells.shrink_to_fit();
        self.creature_cell_of.shrink_to_fit();
        self.aggro_creature_keys.shrink_to_fit();
        self.walking_creature_keys.shrink_to_fit();

        // Per-tick coalescer and scratch buffers. Each is `.clear()`'d at the
        // top of its phase, so calling `shrink_to_fit` here is safe — it
        // doesn't lose any in-flight state, just returns leftover capacity
        // sized for an earlier peak. Without this, a brief 5000-sim spike
        // pins ~150 KB of `scratch_walk_events` capacity indefinitely.
        self.pending_movement.shrink_to_fit();
        self.last_heartbeat_broadcast_tick.shrink_to_fit();
        self.scratch_client_aabb.shrink_to_fit();
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
        mut creatures: Slab<Creature>,
    ) -> Self {
        let mut maps = PathfindingMaps::new();

        // Snap every worlddb creature's z to actual terrain *before* indexing.
        // Mangos rows have stale z for a noticeable percentage of spawns; idle
        // mobs never emit a movement event and so are never snapped at
        // runtime — without this they'd stay floating / underground forever.
        // No-op when pathfinding maps aren't loaded for the creature's map.
        let total = creatures.len();
        let mut snapped = 0_usize;
        for (_, c) in creatures.iter_mut() {
            let z_hint = c.info.position.z;
            if let Some(z) =
                maps.ground_height(c.map, c.info.position.x, c.info.position.y, z_hint)
            {
                c.info.position.z = z;
                snapped += 1;
            }
        }
        tracing::info!(
            "Snapped z to ground for {snapped}/{total} worlddb creatures at spawn"
        );

        let mut creature_by_guid = ahash::AHashMap::with_capacity(creatures.len());
        let mut aggro_creature_keys = Vec::new();
        let mut walking_creature_keys = Vec::with_capacity(creatures.len());
        let mut creature_wander_count = 0;
        let mut creature_waypoint_count = 0;
        let mut creature_cells: ahash::AHashMap<(Map, i32, i32), Vec<usize>> =
            ahash::AHashMap::with_capacity(1024);
        let mut creature_cell_of: ahash::AHashMap<usize, (Map, i32, i32)> =
            ahash::AHashMap::with_capacity(creatures.len());
        for (k, c) in creatures.iter() {
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
            // Seed the spatial grid with every non-Respawning creature.
            // Construction is bulk-fresh so nothing's Respawning yet, but
            // guard anyway for future hygiene.
            if !matches!(c.life_state, CreatureLifeState::Respawning { .. }) {
                let cell = grid_cell_for(c.map, c.info.position.x, c.info.position.y);
                creature_cells.entry(cell).or_default().push(k);
                creature_cell_of.insert(k, cell);
            }
        }
        let mut regions = ahash::AHashMap::new();
        regions.insert(
            Self::WORLD_KEY,
            Arc::new(Mutex::new(RegionState {
                key: Self::WORLD_KEY,
                clients: Slab::new(),
                client_by_guid: ahash::AHashMap::new(),
                creatures,
                creature_by_guid,
                aggro_creature_keys,
                walking_creature_keys,
                creature_wake_at: std::collections::BTreeMap::new(),
                creature_wander_count,
                creature_waypoint_count,
                creature_cells,
                creature_cell_of,
                last_tick_at: None,
                pending_movement: ahash::AHashMap::new(),
                tick_counter: 0,
                last_heartbeat_broadcast_tick: ahash::AHashMap::new(),
                scratch_client_aabb: ahash::AHashMap::new(),
                scratch_walk_events: Vec::new(),
                scratch_to_park: Vec::new(),
                scratch_parked_set: ahash::AHashSet::new(),
                scratch_expired_roots: Vec::new(),
                broadcast_view: Vec::new(),
                pacer: crate::world::TickPacer::new_from_config(
                    &crate::config::config().tick,
                ),
            })),
        );
        Self {
            regions,
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

    /// Variant of [`with_creatures`] that adopts an externally-constructed
    /// `WorldDatabase` (e.g. one restored from a snapshot). The Stage 3
    /// production path uses this so the freshly-loaded DB enters the
    /// `Arc<Mutex<>>` directly rather than being created empty here.
    pub fn with_creatures_and_db(
        clients_waiting_to_join: Receiver<CharacterScreenClient>,
        creatures: Slab<Creature>,
        db: WorldDatabase,
    ) -> Self {
        let mut world = Self::with_creatures(clients_waiting_to_join, creatures);
        world.db = Arc::new(Mutex::new(db));
        world
    }

    /// Build a World suitable for tests and benchmarks: skips pathfinding
    /// map load (so `ground_height` is a noop and there's no filesystem
    /// dependency) and uses synthetic in-memory clients via
    /// [`crate::world::world::client::test_support::synthetic_client`].
    /// Requires an active Tokio runtime — each synthetic client spawns a
    /// writer task.
    ///
    /// `characters` populates the live (in-world) client slab. `creatures`
    /// is indexed the same way `with_creatures` does, minus the
    /// terrain-snap step (positions are taken at face value).
    pub fn for_test(characters: Vec<Character>, creatures: Vec<Creature>) -> Self {
        let maps = PathfindingMaps::new();

        let mut creature_slab: Slab<Creature> = Slab::with_capacity(creatures.len());
        for c in creatures {
            creature_slab.insert(c);
        }
        let mut creature_by_guid = ahash::AHashMap::with_capacity(creature_slab.len());
        let mut aggro_creature_keys = Vec::new();
        let mut walking_creature_keys = Vec::with_capacity(creature_slab.len());
        let mut creature_wander_count = 0;
        let mut creature_waypoint_count = 0;
        let mut creature_cells: ahash::AHashMap<(Map, i32, i32), Vec<usize>> =
            ahash::AHashMap::with_capacity(1024);
        let mut creature_cell_of: ahash::AHashMap<usize, (Map, i32, i32)> =
            ahash::AHashMap::with_capacity(creature_slab.len());
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
                let cell = grid_cell_for(c.map, c.info.position.x, c.info.position.y);
                creature_cells.entry(cell).or_default().push(k);
                creature_cell_of.insert(k, cell);
            }
        }

        // Closed receiver — benches don't push new logins, but the field
        // is non-optional on World.
        let (_tx, clients_waiting_to_join) = tokio::sync::mpsc::channel(1);

        let mut regions = ahash::AHashMap::new();
        let region_state = RegionState {
            key: Self::WORLD_KEY,
            clients: Slab::with_capacity(characters.len()),
            client_by_guid: ahash::AHashMap::with_capacity(characters.len()),
            creatures: creature_slab,
            creature_by_guid,
            aggro_creature_keys,
            walking_creature_keys,
            creature_wake_at: std::collections::BTreeMap::new(),
            creature_wander_count,
            creature_waypoint_count,
            creature_cells,
            creature_cell_of,
            last_tick_at: None,
            pending_movement: ahash::AHashMap::new(),
            tick_counter: 0,
            last_heartbeat_broadcast_tick: ahash::AHashMap::new(),
            scratch_client_aabb: ahash::AHashMap::new(),
            scratch_walk_events: Vec::new(),
            scratch_to_park: Vec::new(),
            scratch_parked_set: ahash::AHashSet::new(),
            scratch_expired_roots: Vec::new(),
            broadcast_view: Vec::new(),
            pacer: crate::world::TickPacer::new_from_config(
                &crate::config::config().tick,
            ),
        };
        let region_arc = Arc::new(Mutex::new(region_state));
        // Seed test characters directly so a no-runtime `for_test` build
        // is ready to tick without going through the login pipeline.
        {
            let region_arc = region_arc.clone();
            // We're synchronous here — `try_lock` won't block because nobody
            // else has the Arc yet.
            let mut region = region_arc.try_lock()
                .expect("freshly-built region must not be locked");
            for character in characters {
                let account = character.account.clone();
                let client = crate::world::world::client::test_support::synthetic_client(
                    character, account,
                );
                region.insert_client(client);
            }
        }
        regions.insert(Self::WORLD_KEY, region_arc);
        Self {
            regions,
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

    /// Walk every in-world client in every region and persist their
    /// `Character` into the snapshot database. Stage 4 implementation:
    /// orchestration via channels.
    ///
    /// Each region's character-collection runs on its own `tokio::spawn`
    /// task; the `JoinHandle` is the one-shot reply channel back to the
    /// orchestrator. With N regions this fans out across the tokio
    /// worker pool — the clone (one `Character` per client) runs in
    /// parallel, and the regions are unblocked from each other while
    /// it happens. The orchestrator collects all replies, then locks
    /// the DB once and writes the aggregated Characters atomically.
    pub async fn sync_clients_to_db(&self) {
        // Stage 4 channel-based collection. Each `tokio::spawn` is the
        // sender; the awaited `JoinHandle` is the receiver. With N
        // regions there are N parallel collections.
        let collection_handles: Vec<tokio::task::JoinHandle<Vec<Character>>> =
            self.regions
                .values()
                .map(|region_arc| {
                    let region_arc = region_arc.clone();
                    tokio::spawn(async move {
                        let region = region_arc.lock().await;
                        region
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
                    "Per-region snapshot collection task panicked: {e}"
                ),
            }
        }

        let mut db = self.db.lock().await;
        for c in all_chars {
            db.replace_character_data(c);
        }
    }

    /// Send a system-channel chat line to every connected in-world client
    /// announcing an adaptive-tickrate transition. Surfacing this in the
    /// chat box lets a GM see backoff/recovery happen live without
    /// tailing the server log. Cheap on the broadcast side — rate
    /// transitions are rare (seconds-to-minutes apart at worst).
    pub async fn broadcast_tick_rate_change(
        &mut self,
        change: crate::world::TickRateChange,
    ) {
        let (label, interval) = match change {
            crate::world::TickRateChange::Backoff { new_interval } => ("backoff", new_interval),
            crate::world::TickRateChange::Recovery { new_interval } => ("recovery", new_interval),
        };
        let hz = 1.0 / interval.as_secs_f32();
        let text = format!(
            "[server] tickrate {label}: {} ms ({:.1} Hz)",
            interval.as_millis(),
            hz
        );
        for region in self.regions.values() {
            let mut region = region.lock().await;
            for (_, c) in region.clients.iter_mut() {
                c.send_system_message(text.clone()).await;
            }
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
        for region in self.regions.values() {
            let mut region = region.lock().await;
            region.shrink_periodic();
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

        // ── Stage 3 region locking ──
        //
        // Each region lives behind a `tokio::sync::Mutex` inside an
        // `Arc`. For the single-region build we clone the Arc here and
        // hold the lock for the duration of this tick — that's
        // uncontended in steady state because nothing else touches the
        // RegionState while a tick is in progress. The per-region
        // `tokio::spawn` block lower down spawns one task per region;
        // with N regions, each task owns its own Arc<Mutex<RegionState>>
        // and they run in parallel on the tokio worker pool. The body
        // here is the orchestrator: drain login, character-screen
        // opcodes, promote (all global), then spawn N region tasks for
        // the per-region phases.
        let primary_region = self.primary_region();
        let mut region_guard = primary_region.lock().await;

        // Lock the shared `db` and `maps` for the duration of the global
        // phases (drain login, char_screen, promote). The per-region tokio
        // tasks spawned afterward re-acquire these locks independently.
        let mut db_guard = self.db.lock().await;
        let mut maps_guard = self.maps.lock().await;

        // Forward-declare values that flow from the global phases into the
        // post-spawn slow-tick log + Tracy block. `tick_dt` and
        // `heartbeat_skip_ratio` moved into the per-region task — they
        // are now per-region values (each region pacer drives its own).
        let t_drain: Duration;
        let t_chrscreen: Duration;
        let t_promote: Duration;

        // Global phases run in a scope so the `region`/`db`/`maps`
        // reborrows of the guards drop before the per-region spawn (which
        // re-acquires the same Arc<Mutex<>>es).
        //
        // Note: per-region `tick_dt`, `last_tick_at`, `tick_counter`,
        // and `heartbeat_skip_ratio` are computed INSIDE the per-region
        // task (using each region's own pacer). The global phase here
        // only handles the truly global work — login drain, character
        // screen, promote. `tick_dt` is no longer forward-declared here.
        let _ = slow_warn; // suppresses a "no longer used at top-level" warning
        {
        let region: &mut RegionState = &mut region_guard;
        let db: &mut WorldDatabase = &mut db_guard;
        // `maps` is currently unused inside the global phases (promote
        // doesn't snap terrain); the lock is held just to keep ordering
        // consistent with the spawned task's re-acquisition.
        let _maps: &mut PathfindingMaps = &mut maps_guard;
        let _ = region; // global phase doesn't read region — Stage 4 moved tick_dt/counter into the spawn task

        {
            let phase = Instant::now();
            let _s = tracing::info_span!("drain_login_queue").entered();
            while let Ok(c) = self.clients_waiting_to_join.try_recv() {
                self.clients_on_character_screen.push(c);
            }
            t_drain = phase.elapsed();
        }

        let phase = Instant::now();
        async {
            for client in self.clients_on_character_screen.iter_mut() {
                handle_character_screen_opcodes(client, db).await;
            }
        }
        .instrument(tracing::info_span!("character_screen_opcodes"))
        .await;
        t_chrscreen = phase.elapsed();

        let phase = Instant::now();
        async {
        // Rebuild the broadcast view from current `region.clients` so the
        // par_iter filters below see fresh data. We rebuild AGAIN later
        // (post-`per_client_loop`) for the flush/AOI phases — that rebuild
        // captures this-tick movement updates. Two builds per tick is
        // sub-ms even at high density; the promote scan savings dwarf it.
        {
            let _s = tracing::info_span!("build_broadcast_view_promote").entered();
            region.broadcast_view.clear();
            region.broadcast_view
                .extend(region.clients.iter().map(|(_, c)| c.broadcast_target()));
        }
        // Hoist the AOI radius once for the entire promote phase. Reused
        // by every `within_aoi_sq` call in the par_iter closures below.
        let aoi_r = crate::config::config().network.aoi_radius_yards;
        let aoi_r_sq = aoi_r * aoi_r;
        // Promoting a player builds an `UpdateObject` for every other client
        // visible from the new player's position. With N bots promoting in
        // one tick this would regenerate the same create-object N times per
        // already-in-world client. Build once per tick and reuse.
        let mut create_object_cache: ahash::AHashMap<Guid, Object> = ahash::AHashMap::new();
        // Cap per-tick promotions so a login burst (e.g. ramping 1k bots
        // in 30 s) doesn't pin a single tick at hundreds of ms while it
        // scans every observer's in-AOI list for each promotion.
        // Configured via `[tick] max_promotions_per_tick`; 0 disables.
        let max_promotions =
            crate::config::config().tick.max_promotions_per_tick;
        let mut promoted_this_tick = 0_u32;
        while let Some(i) = self.clients_on_character_screen
            .iter()
            .position(|a| matches!(a.status, CharacterScreenProgress::WaitingToLogIn(_)))
        {
            if max_promotions > 0 && promoted_this_tick >= max_promotions {
                // Remaining `WaitingToLogIn` clients stay queued — they
                // get picked up next tick. The position() above will
                // find them again.
                break;
            }
            let c = self.clients_on_character_screen.remove(i);
            let guid = match c.status {
                CharacterScreenProgress::WaitingToLogIn(g) => g,
                _ => unreachable!(),
            };
            let Some(character) = db.get_character_by_guid(guid) else {
                tracing::warn!(
                    "Promotion for {} aborted: guid {:?} not found in DB; dropping connection.",
                    c.account_name(),
                    guid
                );
                drop(c);
                continue;
            };
            let mut c = c.into_client(character);

            let new_player_pos = c.character().info.position;
            let new_player_map = c.character().map;

            let new_player_guid = c.character().guid;
            let new_player_object = player_create_object(c.character());
            if let Some(msg) = UpdateObject::from_objects(vec![new_player_object]) {
                msg.broadcast_within_aoi(new_player_pos, new_player_map, &mut region.clients)
                    .await;
                // Seed every AOI observer's visible-entity set with the new
                // player so the upcoming AOI-transition pass doesn't re-emit
                // a duplicate `CreateObject` for them next tick. The filter
                // (map + AOI) is pure-read over `broadcast_view` — par_iter
                // it, then apply the mutation sequentially via the
                // `client_by_guid → slab key` reverse index. At high
                // density the parallel filter is several times faster
                // than the old sequential `region.clients.iter_mut()` scan.
                let observer_guids: Vec<Guid> = region
                    .broadcast_view
                    .par_iter()
                    .filter(|t| {
                        t.map == new_player_map
                            && aoi::within_aoi_sq(&t.position, &new_player_pos, aoi_r_sq)
                    })
                    .map(|t| t.guid)
                    .collect();
                for g in observer_guids {
                    if let Some(&k) = region.client_by_guid.get(&g) {
                        region.clients[k]
                            .session
                            .visible_entities
                            .insert(new_player_guid);
                    }
                }
            }

            let mut visible_objects: Vec<Object> = Vec::new();
            let mut movement_starts: Vec<MSG_MOVE_START_FORWARD_Server> = Vec::new();

            // Parallel filter: which existing players are in the new
            // player's AOI? This is the dominant cost of promote at
            // high density — par_iter across rayon threads brings each
            // promote from a multi-ms slab walk down to a fraction of
            // that. `broadcast_view` was rebuilt at the top of the
            // phase and re-extended after every `insert_client` below,
            // so it reflects every player already in the world plus
            // everyone promoted earlier in this same tick.
            let candidate_guids: Vec<Guid> = region
                .broadcast_view
                .par_iter()
                .filter(|t| {
                    t.guid != new_player_guid
                        && t.map == new_player_map
                        && aoi::within_aoi_sq(&t.position, &new_player_pos, aoi_r_sq)
                })
                .map(|t| t.guid)
                .collect();
            // Sequential build: per-tick `create_object_cache` and the
            // new player's own `visible_entities` are mutated here, so
            // this stays single-threaded. The cache provides cross-
            // promotion memoization — N promotes in the same tick share
            // mask construction for overlapping visibility sets.
            for other_guid in candidate_guids {
                let Some(&other_key) = region.client_by_guid.get(&other_guid) else {
                    continue;
                };
                let client = &region.clients[other_key];
                let obj = create_object_cache
                    .entry(other_guid)
                    .or_insert_with(|| player_create_object(client.character()))
                    .clone();
                visible_objects.push(obj);
                c.session.visible_entities.insert(other_guid);
                // If this player is mid-motion, also queue a movement-start
                // so the new client animates them instead of seeing a
                // stationary object that teleports on every heartbeat.
                if client.character().info.flags.get_forward() {
                    movement_starts.push(MSG_MOVE_START_FORWARD_Server {
                        guid: other_guid,
                        info: client.character().info.clone(),
                    });
                }
            }

            // Spatial-grid creature scan: only check creatures in the
            // 3×3 cell window around the new player's position. The
            // grid (`creature_cells`) holds only Alive + Corpse
            // creatures — Respawning ones are removed on Corpse→Respawn
            // and re-inserted on Respawn→Alive, so this also fixes a
            // latent bug where the previous full-slab scan would emit
            // CreateObject for in-AOI Respawning creatures the client
            // shouldn't yet see.
            //
            // Cell size (250 yd) > AOI radius (200 yd default), so the
            // 3×3 window is guaranteed to cover every creature within
            // AOI of the anchor. Same pattern used in
            // `tick_aoi_transitions`'s creature scan.
            {
                let cx = (new_player_pos.x / CREATURE_GRID_CELL_YD).floor() as i32;
                let cy = (new_player_pos.y / CREATURE_GRID_CELL_YD).floor() as i32;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        let Some(keys) = region
                            .creature_cells
                            .get(&(new_player_map, cx + dx, cy + dy))
                        else {
                            continue;
                        };
                        for &ck in keys {
                            let creature = &region.creatures[ck];
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
            // Chunked send: at high density `visible_objects` holds
            // thousands of CreateObject2s. One packet exceeds the
            // wire-protocol u16 size header → silent truncation →
            // ARC4 desync → client appears in-world but is dead-on-the-
            // wire. This was the "log out + log back in at high
            // density = frozen client" bug. See `UpdateObject::send_chunked`.
            UpdateObject::send_chunked(visible_objects, &mut c).await;
            for start in movement_starts {
                c.send_message(start).await;
            }
            tracing::debug!(
                "promote: account={} name={} pos=({:.1},{:.1},{:.1}) map={:?} -> sent {} CreateObjects + {} MoveStarts; clients_in_world={} creatures={}",
                c.session.account_name,
                c.character().name,
                new_player_pos.x,
                new_player_pos.y,
                new_player_pos.z,
                new_player_map,
                visible_count,
                starts_count,
                region.clients.len(),
                region.creatures.len(),
            );

            // Snapshot the new player's BroadcastTarget BEFORE
            // `insert_client` moves `c` into the slab. Appending to
            // `broadcast_view` here means the NEXT promotion in this
            // same tick can see this player via its par_iter filter —
            // matches the pre-change behavior where `region.clients` was
            // iterated directly and reflected each insert immediately.
            let new_target = c.broadcast_target();
            region.insert_client(c);
            region.broadcast_view.push(new_target);
            promoted_this_tick += 1;
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
        } // end of global-phase scope: region/db/maps reborrows drop here

        // ── Stage 3: per-region tick on its own tokio task ──
        //
        // Release the orchestrator's region/db/maps guards (held for the
        // global drain/char_screen/promote phases above), then spawn one
        // task per region. Each task re-acquires the locks it needs and
        // runs the per-region phases (per_client_loop, flush, AOI,
        // apply_cmds, corpses, creature_ai, drain_logouts, stale-cleanup)
        // independently. With one region we await one task; with N
        // regions, N tasks run in parallel on the tokio worker pool.
        // Departed clients (logouts + region transitions) are collected
        // into the task's `__departed` and pushed back into
        // `self.clients_on_character_screen` by the orchestrator after
        // the task completes.
        drop(region_guard);
        drop(maps_guard);
        drop(db_guard);

        let mut per_region_handles: Vec<tokio::task::JoinHandle<PerRegionTickResult>> = Vec::new();
        for region_arc in self.regions.values() {
            let region_arc = region_arc.clone();
            let db_arc = self.db.clone();
            let maps_arc = self.maps.clone();
            per_region_handles.push(tokio::spawn(async move {
                let mut region_guard = region_arc.lock().await;
                let region: &mut RegionState = &mut region_guard;

                // ── Per-region pacer: decide whether to tick this round ──
                //
                // The global orchestrator runs at `target_interval_ms`
                // (default 33 ms = 30 Hz). Each region's pacer carries
                // its own `current_interval` which the pacer's
                // adaptive backoff stretches to 66 → 132 → … →
                // `max_interval_ms` (1 s = 1 Hz) under sustained
                // slow ticks. If less than `current_interval` has
                // elapsed since this region's last tick, the region
                // skips this global round entirely — the spawn
                // returns a "skipped" result with empty fields and
                // the orchestrator suppresses per-region Tracy/log
                // emission for it. The region's `last_tick_at` is
                // only advanced when work actually runs.
                let now = std::time::Instant::now();
                let due = region
                    .last_tick_at
                    .map(|t| now.duration_since(t) >= region.pacer.current_interval)
                    .unwrap_or(true);
                if !due {
                    return PerRegionTickResult {
                        region_key: region.key,
                        skipped: true,
                        t_per_client: Duration::ZERO,
                        t_build_view: Duration::ZERO,
                        t_flush: Duration::ZERO,
                        t_aoi: Duration::ZERO,
                        t_apply_cmds: Duration::ZERO,
                        t_corpses: Duration::ZERO,
                        t_creatures: Duration::ZERO,
                        t_logouts: Duration::ZERO,
                        t_region_total: Duration::ZERO,
                        departed: Vec::new(),
                        clients_count: region.clients.len(),
                        creatures_count: region.creatures.len(),
                        creature_idle_count: 0,
                        creature_wander_count: region.creature_wander_count,
                        creature_waypoint_count: region.creature_waypoint_count,
                        creature_aggro_count: region.aggro_creature_keys.len(),
                        walking_creature_count: region.walking_creature_keys.len(),
                    };
                }

                // Compute wall-clock dt since this region's last tick.
                // Clamp at 1 s so a frozen tick doesn't blow the
                // auto-attack timer negative; clamp at the pacer's
                // current_interval as a sanity floor.
                let tick_dt: f32 = region
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
                region.last_tick_at = Some(now);
                region.tick_counter = region.tick_counter.wrapping_add(1);

                // Per-region heartbeat throttle: pacer-driven instead
                // of global slow_warn. With pacer at 33 ms the ratio
                // is 1 (no throttle); at 100 ms it's 3 (every 3rd
                // heartbeat). Floors at 1.
                let heartbeat_skip_ratio: u64 = {
                    let target_ms = crate::config::config().tick.target_interval_ms.max(1);
                    let current_ms = region.pacer.current_interval.as_millis() as u64;
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
        let client_keys: Vec<usize> = region.clients.iter().map(|(k, _)| k).collect();
        for key in client_keys {
            let mut client = region.remove_client(key);
            // Per-iteration: the opcode handler may flip this true for the
            // CURRENT client (CMSG_LOGOUT_REQUEST). Resetting on every iter
            // is load-bearing — a single declaration above the loop would
            // be sticky across clients and one logout would drag every
            // later client in the slab into character-screen with it.
            let mut move_to_character_screen = false;
            let mut entities = Entities::new(
                &mut region.clients,
                &region.client_by_guid,
                &mut region.creatures,
                &region.creature_by_guid,
                &mut region.pending_movement,
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
                } else if let Some(&ck) = region.creature_by_guid.get(&target_guid) {
                    let cr = &region.creatures[ck];
                    if cr.map == attacker_map {
                        Some((
                            SwingKind::Creature(ck),
                            cr.info.position,
                            world_opcode_handler::combat::is_moving(&cr.info),
                        ))
                    } else {
                        None
                    }
                } else {
                    // Player target — O(N) scan over clients. At 1000 PvP
                    // bots this is 1M comparisons/tick which is well under
                    // budget; if it ever bites perf, add a guid → slab key
                    // reverse index alongside `creature_by_guid`.
                    region.clients
                        .iter()
                        .find(|(_, c)| {
                            c.character().guid == target_guid
                                && !c.character().is_dead()
                                && c.character().map == attacker_map
                        })
                        .map(|(k, c)| {
                            (
                                SwingKind::Player(k),
                                c.character().info.position,
                                world_opcode_handler::combat::is_moving(&c.character().info),
                            )
                        })
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
                    let new_key = region.insert_client(client);
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
                        &mut region.clients,
                    )
                    .await;

                    match kind {
                        SwingKind::Creature(creature_key) => {
                            let creature = &mut region.creatures[creature_key];
                            creature.health = creature.health.saturating_sub(swing_damage);
                            let creature_map = creature.map;
                            let creature_pos = creature.info.position;
                            let creature_guid = creature.guid;
                            let killed = creature.health == 0;

                            if killed {
                                region.kill_creature(creature_key).await;
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
                                    &mut region.clients,
                                )
                                .await;
                            }
                        }
                        SwingKind::Player(target_key) => {
                            let target = &mut region.clients[target_key];
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
                            // Target is in `region.clients` so they'll receive via
                            // broadcast; attacker is held outside, send directly.
                            client.send_message(hp_update.clone()).await;
                            aoi::broadcast_within_aoi(
                                hp_update,
                                target_pos,
                                target_map,
                                &mut region.clients,
                            )
                            .await;
                        }
                    }
                }
            }

            if move_to_character_screen {
                keys_to_move_to_character_screen.push(key);
            }

            let new_key = region.insert_client(client);
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
            region.broadcast_view.clear();
            region.broadcast_view
                .extend(region.clients.iter().map(|(_, c)| c.broadcast_target()));
        }
        let t_build_view = phase.elapsed();
        if let Some(client) = tracy_client::Client::running() {
            client.plot(
                tracy_client::plot_name!("broadcast_view_len"),
                region.broadcast_view.len() as f64,
            );
        }

        // Flush coalesced movement broadcasts. Each entry was queued by a
        // movement opcode handler this tick; we issue at most one broadcast
        // per source per tick via the serialize-once `broadcast_opcode_within_aoi`
        // path. The map is reused across ticks — `.drain()` keeps capacity.
        //
        // Crucially we pass `Some(source_guid)` so the source player does NOT
        // receive their own movement opcode back: at this point the source is
        // back in `region.clients` (per_client_loop re-inserted them), and any
        // echo would be treated by the local client as a server position
        // correction — visible as rubber-band / "laggy movement" on the
        // player's own character.
        let phase = Instant::now();
        {
            let _s = tracing::info_span!("flush_movement_broadcasts").entered();
            // Per-tick broadcast totals so Tracy can show whether the
            // movement broadcast leg is a hotspot. `sources` is the
            // number of distinct players that moved this tick (after
            // `pending_movement` coalescing), `recipients` is the sum
            // of per-source observer counts, `bytes` is recipients *
            // per-frame byte length — total egress this tick on the
            // movement-broadcast path.
            let mut sources = 0_usize;
            let mut recipients = 0_usize;
            let mut bytes = 0_usize;
            let mut throttled = 0_usize;
            // Borrow these as raw fields up front — the loop body takes
            // `&mut region.clients` which conflicts with `&region.tick_counter`
            // under the standard borrow check.
            let tick_counter = region.tick_counter;
            let skip_ratio = heartbeat_skip_ratio;
            for (source_guid, pm) in region.pending_movement.drain() {
                // Heartbeat-throttle path: under pacer-detected load, emit
                // a periodic `MSG_MOVE_HEARTBEAT_Server` only every
                // `skip_ratio` ticks per source. Transition opcodes
                // (start/stop/strafe/jump/...) always fan out — clients
                // can't infer those locally and skipping them would
                // visibly desync remote players.
                let is_heartbeat = matches!(
                    pm.msg,
                    ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(_)
                );
                if is_heartbeat && skip_ratio > 1 {
                    let last = region
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
                    &region.broadcast_view,
                );
                sources += 1;
                recipients += r;
                bytes += r * b;
                // Stamp the throttle map AFTER a successful emit (both
                // heartbeats and transitions). Stamping on transitions
                // too means the next periodic heartbeat is delayed by
                // `skip_ratio` ticks rather than potentially firing one
                // tick later.
                region.last_heartbeat_broadcast_tick
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
        t_flush = phase.elapsed();

        // AOI transitions: for each connected player, diff their previously
        // visible set against the players currently within `AOI_RADIUS_YARDS`
        // on the same map. Anything that left → `OutOfRangeObjects`
        // (despawn). Anything that entered → `CreateObject2` (spawn).
        // Without this pass, players who walk past the AOI boundary
        // linger forever on observers' clients as motionless ghosts.
        let phase = Instant::now();
        let aoi_stats = region.tick_aoi_transitions().await;
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
        region.apply_commands(&mut commands, &mut *maps).await;
        t_apply_cmds = phase.elapsed();

        let phase = Instant::now();
        region.tick_corpses_and_respawns().await;
        t_corpses = phase.elapsed();

        let phase = Instant::now();
        region.tick_creature_ai(&mut *maps).await;
        t_creatures = phase.elapsed();

        let phase = Instant::now();
        async {
        for key in keys_to_move_to_character_screen {
            let c = region.remove_client(key);
            let logout_pos = c.character().info.position;
            let logout_map = c.character().map;
            let logout_guid = c.character().guid;
            // Drop the heartbeat-throttle bookkeeping for the leaving
            // player so the map doesn't accumulate stale guids over a
            // long server lifetime.
            region.last_heartbeat_broadcast_tick.remove(&logout_guid);
            for (_, a) in &mut region.clients {
                if a.character().map == logout_map
                    && aoi::within_aoi(&a.character().info.position, &logout_pos)
                {
                    a.send_message(SMSG_DESTROY_OBJECT { guid: logout_guid })
                        .await;
                }
                // Drop the logged-out guid from every observer's
                // visible_entities (regardless of AOI distance — they may
                // have had the guid cached from a recent close pass). This
                // keeps the next AOI-transition diff from spuriously
                // re-emitting `OutOfRangeObjects` for an already-handled
                // logout.
                a.session.visible_entities.remove(&logout_guid);
            }

            let c = c.into_character_screen_client();
            __departed.push(c);
        }
        }
        .instrument(tracing::info_span!("drain_logouts"))
        .await;
        t_logouts = phase.elapsed();

        let stale_client_keys: Vec<usize> = region
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
            let c = region.remove_client(key);
            let logout_map = c.character().map;
            let guid = c.character().guid;
            region.last_heartbeat_broadcast_tick.remove(&guid);
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
            for (_, c) in region.clients.iter_mut() {
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
                let creature_aggro_count = region.aggro_creature_keys.len();
                let creature_wander_count = region.creature_wander_count;
                let creature_waypoint_count = region.creature_waypoint_count;
                let walking_creature_count = region.walking_creature_keys.len();
                let creature_idle_count = region.creatures.len()
                    .saturating_sub(creature_wander_count + creature_waypoint_count + creature_aggro_count);

                // Sum the per-region phase timings into the per-region
                // total. Used both for the `region_tick_ms` Tracy plot
                // and for the per-region pacer's adaptive backoff
                // signal.
                let t_region_total = t_per_client + t_build_view + t_flush + t_aoi
                    + t_apply_cmds + t_corpses + t_creatures + t_logouts;

                // Per-region pacer: feed this tick's per-region cost
                // and publish the resulting state to the process-wide
                // snapshot so `.regions` can show it. Today the actual
                // sleep happens at the global level in `run_world`;
                // when long-lived per-region task loops land, each
                // task will sleep on its own `pacer.current_interval`.
                let (_sleep_for, _change) = region.pacer.observe(t_region_total);
                crate::world::region::publish_pacer_state(
                    region.key,
                    crate::world::region::PacerSnapshot {
                        current_interval_ms: region.pacer.current_interval.as_millis() as u64,
                        slow_ema: region.pacer.slow_ema,
                        healthy_streak: region.pacer.healthy_streak,
                        last_tick_ms: t_region_total.as_millis() as u64,
                    },
                );

                // Per-region Tracy plot — `region_tick_ms` is the
                // total per-region cost for this tick. Emitted from
                // inside the task so it sits on the same Tracy
                // timeline as the per-region phase plots.
                if let Some(client) = tracy_client::Client::running() {
                    client.plot(
                        tracy_client::plot_name!("region_tick_ms"),
                        t_region_total.as_secs_f64() * 1000.0,
                    );
                }

                PerRegionTickResult {
                    region_key: region.key,
                    skipped: false,
                    t_per_client,
                    t_build_view,
                    t_flush,
                    t_aoi,
                    t_apply_cmds,
                    t_corpses,
                    t_creatures,
                    t_logouts,
                    t_region_total,
                    departed: __departed,
                    clients_count: region.clients.len(),
                    creatures_count: region.creatures.len(),
                    creature_idle_count,
                    creature_wander_count,
                    creature_waypoint_count,
                    creature_aggro_count,
                    walking_creature_count,
                }
            }));
        }

        // Await all per-region tasks. With one region this is one await;
        // with N regions each ran on a tokio worker thread in parallel.
        // The orchestrator pulls departed clients out of each result
        // (logouts → char_screen) and keeps the rest of the result for
        // post-spawn metrics. Skipped regions return cheap zero-valued
        // results; we filter them out of Tracy/log emission below.
        let mut all_results: Vec<PerRegionTickResult> = Vec::new();
        for handle in per_region_handles {
            match handle.await {
                Ok(mut r) => {
                    self.clients_on_character_screen
                        .extend(std::mem::take(&mut r.departed));
                    all_results.push(r);
                }
                Err(e) => tracing::error!("Per-region tick task panicked: {e}"),
            }
        }
        // Prefer the first non-skipped result for Tracy plots and the
        // slow-tick log; fall back to any result if every region
        // skipped (so e.g. `regions_active` plot still emits).
        let per_region_result_active = all_results.iter().find(|r| !r.skipped);
        // Reference into all_results; we only need it for reads below.
        let per_region_result = per_region_result_active
            .or_else(|| all_results.first())
            .expect("at least one region must tick per global tick");
        // Pull out the per-region phase timings + counts so the
        // orchestrator's slow-tick log and Tracy block can use them.
        let t_per_client = per_region_result.t_per_client;
        let t_build_view = per_region_result.t_build_view;
        let t_flush = per_region_result.t_flush;
        let t_aoi = per_region_result.t_aoi;
        let t_apply_cmds = per_region_result.t_apply_cmds;
        let t_corpses = per_region_result.t_corpses;
        let t_creatures = per_region_result.t_creatures;
        let t_logouts = per_region_result.t_logouts;
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
                per_region_result.clients_count as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures"),
                per_region_result.creatures_count as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures_idle"),
                per_region_result.creature_idle_count as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures_wander"),
                per_region_result.creature_wander_count as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures_waypoint"),
                per_region_result.creature_waypoint_count as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures_aggro"),
                per_region_result.creature_aggro_count as f64,
            );
            client.plot(
                tracy_client::plot_name!("char_screen_clients"),
                self.clients_on_character_screen.len() as f64,
            );
            client.plot(
                tracy_client::plot_name!("tick_ms"),
                tick_start.elapsed().as_secs_f64() * 1000.0,
            );
            // Only emit per-region phase plots when the picked
            // `per_region_result` actually did work — a region that
            // skipped this global tick reports zeros, which would
            // dilute the dashboard.
            if !per_region_result.skipped {
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
            // Briefly re-lock maps to read the ADT counter. The per-region
            // tasks already released their maps lock by now, so this is
            // uncontended in the single-region build.
            let adt_count = {
                let maps = self.maps.lock().await;
                maps.attempted_adt_count()
            };
            client.plot(
                tracy_client::plot_name!("adt_tiles_loaded"),
                adt_count as f64,
            );

            // ── Stage 4: region + cross-region observability ──
            //
            // `regions_active` is the count of `World::regions` entries
            // (today: always 1; once Stage 3 partition lands each spatial
            // region becomes its own entry). `region_max_clients` is the
            // busiest region's client count — useful for spotting hot
            // spots once partition is on. The three cross_region plots
            // drain process-wide atomic counters once per tick: they
            // stay at 0 until the routing table holds neighbor inboxes,
            // i.e. when Stage 3 actually spins up parallel region tasks.
            // Adding the plots now means dashboards line up the moment
            // partition lands.
            client.plot(
                tracy_client::plot_name!("regions_active"),
                self.regions.len() as f64,
            );
            client.plot(
                tracy_client::plot_name!("region_max_clients"),
                per_region_result.clients_count as f64,
            );
            client.plot(
                tracy_client::plot_name!("cross_region_emitted"),
                crate::world::region::drain_counter(
                    &crate::world::region::CROSS_REGION_EMITTED,
                ) as f64,
            );
            client.plot(
                tracy_client::plot_name!("cross_region_dropped"),
                crate::world::region::drain_counter(
                    &crate::world::region::CROSS_REGION_DROPPED,
                ) as f64,
            );
            client.plot(
                tracy_client::plot_name!("cross_region_drained"),
                crate::world::region::drain_counter(
                    &crate::world::region::CROSS_REGION_DRAINED,
                ) as f64,
            );
            client.frame_mark();
        }

        // If the tick blew its budget, print where the time went. One line,
        // sortable on the longest column. Lets the operator diagnose without
        // standing up Tracy. The budget is whatever `TickPacer` has settled
        // on — at 10 Hz it's 100 ms; under sustained overload the pacer may
        // halve us to 200 ms or further, and the WARN threshold scales with
        // it so we don't spam log lines for ticks that are slow only relative
        // to the original target.
        let total = tick_start.elapsed();
        if total > slow_warn {
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            // If every region skipped this round (rare — usually
            // happens only on the very first tick of a heavily backed-
            // off region) the per-region timings are zero. Log without
            // the per-region columns in that case.
            if per_region_result.skipped {
                tracing::warn!(
                    target: "tick_slow",
                    "slow tick (all regions skipped) total={:.1}ms drain={:.1} chrscreen={:.1} promote={:.1}",
                    ms(total),
                    ms(t_drain),
                    ms(t_chrscreen),
                    ms(t_promote),
                );
            } else {
                tracing::warn!(
                    target: "tick_slow",
                    "slow tick region={} total={:.1}ms drain={:.1} chrscreen={:.1} promote={:.1} per_client={:.1} build_view={:.1} flush={:.1} aoi={:.1} apply={:.1} corpses={:.1} creatures={:.1} logouts={:.1} | clients={} creatures_active={}",
                    per_region_result.region_key,
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
                    per_region_result.clients_count,
                    per_region_result.walking_creature_count,
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

    v.push(
        SMSG_INITIAL_SPELLS {
            unknown1: 0,
            initial_spells: character
                .race_class
                .starter_spells()
                .iter()
                .map(|a| InitialSpell {
                    spell_id: *a as u16,
                    unknown1: 0,
                })
                .collect(),
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
        let (_, cx, cy) = grid_cell_for(Map::EasternKingdoms, 0.0, 0.0);
        assert_eq!((cx, cy), (0, 0));
    }

    #[test]
    fn grid_cell_for_positive_inside_first_cell() {
        // (1, 1) and (CELL-0.01, CELL-0.01) both land in cell (0, 0).
        let (_, cx, cy) = grid_cell_for(Map::EasternKingdoms, 1.0, 1.0);
        assert_eq!((cx, cy), (0, 0));
        let (_, cx, cy) = grid_cell_for(
            Map::EasternKingdoms,
            CREATURE_GRID_CELL_YD - 0.01,
            CREATURE_GRID_CELL_YD - 0.01,
        );
        assert_eq!((cx, cy), (0, 0));
    }

    #[test]
    fn grid_cell_for_boundary_at_cell_size_jumps_to_next_cell() {
        // Exactly CELL_YD on the X axis is in cell 1, not 0. This is the
        // classic floor-vs-truncate trap: `as i32` truncates toward zero,
        // so a naive `(x / CELL) as i32` would land 249.99 → 0 (correct)
        // and 250.00 → 1 (correct), BUT -0.01 → 0 (wrong — should be -1).
        // The explicit `.floor()` is what makes negatives behave.
        let (_, cx, _) = grid_cell_for(Map::EasternKingdoms, CREATURE_GRID_CELL_YD, 0.0);
        assert_eq!(cx, 1);
        let (_, _, cy) = grid_cell_for(Map::EasternKingdoms, 0.0, CREATURE_GRID_CELL_YD);
        assert_eq!(cy, 1);
    }

    #[test]
    fn grid_cell_for_small_negative_lands_in_cell_minus_one() {
        // Regression guard for the truncate-vs-floor footgun (see comment
        // above). Without `.floor()`, this returned (0, 0) — wrong, and
        // would silently put creatures into the wrong neighbor cell.
        let (_, cx, cy) = grid_cell_for(Map::EasternKingdoms, -0.01, -0.01);
        assert_eq!((cx, cy), (-1, -1));
    }

    #[test]
    fn grid_cell_for_preserves_map() {
        let (m, _, _) = grid_cell_for(Map::Kalimdor, 100.0, 200.0);
        assert_eq!(m, Map::Kalimdor);
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
}
