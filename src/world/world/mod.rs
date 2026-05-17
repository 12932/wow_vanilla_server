use crate::world::aoi;
use crate::world::character_screen_handler::handle_character_screen_opcodes;
use crate::world::database::WorldDatabase;
use crate::world::update_object::UpdateObject;
use crate::world::world::client::Client;
use crate::world::world::pathfinding_maps::PathfindingMaps;
use crate::world::world_opcode_handler;
use crate::world::world_opcode_handler::character::Character;
use crate::world::world_opcode_handler::creature::{
    Creature, CreatureBehavior, CreatureLifeState, WALK_SPEED,
};
use crate::world::world_opcode_handler::entities::Entities;
use crate::world::world_opcode_handler::simulated_player::SimulatedPlayer;
use client::character_screen_client::{CharacterScreenClient, CharacterScreenProgress};
use slab::Slab;
use std::time::{Duration, Instant};
use tracing::Instrument;
use tokio::sync::mpsc::Receiver;
use wow_world_base::combat::UNARMED_SPEED;
use wow_world_base::movement::{
    DEFAULT_RUNNING_BACKWARDS_SPEED, DEFAULT_RUNNING_SPEED, DEFAULT_TURN_SPEED,
};
use wow_world_base::vanilla::position::Position;
use wow_world_base::vanilla::{HitInfo, Map, SplineFlag};
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::UpdateMask;
use wow_world_messages::vanilla::{
    DamageInfo, InitialSpell, Language, MSG_MOVE_HEARTBEAT_Server, MSG_MOVE_JUMP_Server,
    MSG_MOVE_SET_FACING_Server, MSG_MOVE_START_FORWARD_Server, MSG_MOVE_STOP_Server,
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

#[derive(Debug)]
pub struct World {
    clients: Slab<Client>,
    clients_on_character_screen: Vec<CharacterScreenClient>,
    clients_waiting_to_join: Receiver<CharacterScreenClient>,

    creatures: Slab<Creature>,
    /// Reverse index from creature guid to slab key. Must be maintained in
    /// lockstep with `creatures` on every insert/remove.
    creature_by_guid: ahash::AHashMap<Guid, usize>,
    /// Slab keys of `AggroChase` creatures (typically `.spawn`'d GM mobs).
    /// Maintained on insert/remove; `tick_creature_ai` iterates this directly
    /// instead of scanning all 51k creatures.
    aggro_creature_keys: Vec<usize>,
    /// Slab keys of `RandomWander` + `Waypoint` creatures **currently active**
    /// — the ones `tick_walking_creatures` iterates. Creatures that are
    /// idling out a `next_decision_at` / `idle_until` window get removed and
    /// pushed into `creature_wake_at`, then re-inserted when their wake time
    /// elapses. Keeps the per-tick walking iteration tiny even when 50 k mobs
    /// are loaded.
    walking_creature_keys: Vec<usize>,
    /// Parked walking creatures keyed by the time they should re-enter
    /// `walking_creature_keys`. BTreeMap so the next-to-wake entry is `O(1)`.
    /// Drained from the front at the top of `tick_walking_creatures`. Stale
    /// keys (e.g. for creatures that died while parked) are filtered on
    /// drain rather than purged on kill.
    creature_wake_at: std::collections::BTreeMap<Instant, Vec<usize>>,
    /// Per-behavior counts published to Tracy each tick; kept current via
    /// `register_creature` / `unregister_creature`.
    creature_wander_count: usize,
    creature_waypoint_count: usize,
    simulated_players: Slab<SimulatedPlayer>,
    /// Reverse index from sim guid to slab key. Maintained in lockstep with
    /// `simulated_players` on every insert/remove (same pattern as
    /// `creature_by_guid`).
    simulated_by_guid: ahash::AHashMap<Guid, usize>,

    maps: PathfindingMaps,

    last_packet_sample: u64,
    last_packet_sample_at: Instant,

    /// Start of the previous tick, used to compute wall-clock `dt` for time-
    /// dependent state like `auto_attack_timer`. `None` on the very first
    /// tick; falls back to `crate::world::TARGET_INTERVAL` then.
    last_tick_at: Option<Instant>,

    /// Per-tick movement coalescer: at most one outbound movement broadcast
    /// per source player per tick. Keyed by the source's `Guid`; later
    /// opcodes from the same source replace earlier ones (HEARTBEAT and
    /// state-transition opcodes carry the full `MovementInfo`, so the
    /// latest is always correct for observers). Drained in
    /// `flush_movement_broadcasts`. Held on `World` so the underlying
    /// allocations are reused tick over tick.
    pending_movement: ahash::AHashMap<Guid, PendingMovement>,

    // ── Per-tick scratch buffers ──
    // Held on `World` rather than declared as locals inside the tick phases
    // so the underlying Vec/HashMap allocations are reused tick-over-tick.
    // Each scratch is `.clear()`d at the top of its phase and refilled.
    scratch_client_aabb: ahash::AHashMap<Map, (f32, f32, f32, f32)>,
    scratch_walk_events: Vec<(usize, Vector3d, Map, CreatureMoveEvent)>,
    scratch_to_park: Vec<(Instant, usize)>,
    scratch_parked_set: ahash::AHashSet<usize>,
    scratch_expired_roots: Vec<(Guid, Map, Vector3d, MovementInfo)>,
}

#[derive(Debug)]
pub(crate) struct PendingMovement {
    pub msg: ServerOpcodeMessage,
    pub anchor: Vector3d,
    pub map: Map,
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
        }
        Self {
            clients: Slab::new(),
            clients_on_character_screen: vec![],
            clients_waiting_to_join,
            creatures,
            creature_by_guid,
            aggro_creature_keys,
            walking_creature_keys,
            creature_wake_at: std::collections::BTreeMap::new(),
            creature_wander_count,
            creature_waypoint_count,
            simulated_players: Slab::new(),
            simulated_by_guid: ahash::AHashMap::new(),
            maps,
            last_packet_sample: 0,
            last_packet_sample_at: Instant::now(),
            last_tick_at: None,
            pending_movement: ahash::AHashMap::new(),
            scratch_client_aabb: ahash::AHashMap::new(),
            scratch_walk_events: Vec::new(),
            scratch_to_park: Vec::new(),
            scratch_parked_set: ahash::AHashSet::new(),
            scratch_expired_roots: Vec::new(),
        }
    }

    /// Add a freshly-inserted creature to the behavior key indexes.
    fn register_creature(&mut self, key: usize) {
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
    }

    pub fn sync_clients_to_db(&self, db: &mut WorldDatabase) {
        for (_, client) in &self.clients {
            db.replace_character_data(client.character().clone());
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
        for (_, c) in self.clients.iter_mut() {
            c.send_system_message(text.clone()).await;
        }
    }

    /// Return excess capacity in the long-lived slabs / hash maps / scratch
    /// buffers to the allocator. Vec / Slab / HashMap don't auto-shrink on
    /// remove or `.clear()`, so over a long run with peak-load bursts
    /// (`.simulate 5000`, 1000-bot loadtest, mass invasion) the underlying
    /// buffers hold significantly more memory than the live entries justify.
    /// Called from `run_world` once per snapshot save (~60s) which is well
    /// outside any hot path.
    pub fn shrink_periodic(&mut self) {
        // Long-lived primary collections.
        self.clients.shrink_to_fit();
        self.creatures.shrink_to_fit();
        self.simulated_players.shrink_to_fit();
        self.creature_by_guid.shrink_to_fit();
        self.simulated_by_guid.shrink_to_fit();
        self.aggro_creature_keys.shrink_to_fit();
        self.walking_creature_keys.shrink_to_fit();
        self.clients_on_character_screen.shrink_to_fit();

        // Per-tick coalescer and scratch buffers. Each is `.clear()`'d at the
        // top of its phase, so calling `shrink_to_fit` here is safe — it
        // doesn't lose any in-flight state, just returns leftover capacity
        // sized for an earlier peak. Without this, a brief 5000-sim spike
        // pins ~150 KB of `scratch_walk_events` capacity indefinitely.
        self.pending_movement.shrink_to_fit();
        self.scratch_client_aabb.shrink_to_fit();
        self.scratch_walk_events.shrink_to_fit();
        self.scratch_to_park.shrink_to_fit();
        self.scratch_parked_set.shrink_to_fit();
        self.scratch_expired_roots.shrink_to_fit();
        // `creature_wake_at` is a BTreeMap — no shrink_to_fit on the node
        // allocator. Entries naturally drain at their wake time so peak
        // capacity isn't held indefinitely the way Vec/HashMap capacity is.
    }

    /// Transition a live creature to the corpse state: zero health, record
    /// time of death, halve the respawn delay if the mob lived for less than
    /// its current delay, de-index from the AI behavior key lists (so it
    /// stops ticking), and broadcast the visual death state to AOI viewers.
    /// Keeps the creature in the slab + `creature_by_guid` so queries still
    /// resolve while it's lying around.
    async fn kill_creature(&mut self, key: usize) {
        let Some(creature) = self.creatures.get_mut(key) else {
            return;
        };
        // Don't re-kill an already-dead corpse.
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

        // Tell viewers the unit is dead: health=0 + DEAD stand state. The
        // client plays the death animation and leaves the corpse on the
        // ground until we broadcast SMSG_DESTROY_OBJECT after `CORPSE_DESPAWN`.
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

    /// Drop a creature key from the AI behavior buckets without touching the
    /// slab or `creature_by_guid`. Used when transitioning to a corpse.
    fn mark_creature_dead(&mut self, key: usize) {
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
                self.creature_waypoint_count = self.creature_waypoint_count.saturating_sub(1);
            }
            CreatureBehavior::Idle => {}
        }
    }

    /// Re-add a respawned creature's key to its behavior bucket.
    fn unmark_creature_dead(&mut self, key: usize) {
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
    /// whose timer has elapsed transition back to `Alive` (broadcast a fresh
    /// create-object, restore HP, snap back to spawn position).
    #[tracing::instrument(level = "info", skip_all, name = "tick_corpses_and_respawns")]
    async fn tick_corpses_and_respawns(&mut self) {
        let now = Instant::now();

        // Decide transitions without holding a mutable borrow on the slab.
        let mut to_decay: Vec<usize> = Vec::new();
        let mut to_revive: Vec<usize> = Vec::new();
        for (key, c) in self.creatures.iter() {
            match c.life_state {
                CreatureLifeState::Corpse { died_at } => {
                    if now.saturating_duration_since(died_at)
                        >= crate::world::world_opcode_handler::creature::CORPSE_DESPAWN
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
            let destroy = SMSG_DESTROY_OBJECT { guid };
            aoi::broadcast_within_aoi(destroy, pos, map, &mut self.clients).await;
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
            self.unmark_creature_dead(key);
            if let Some(msg) = UpdateObject::from_objects(vec![create_object]) {
                msg.broadcast_within_aoi(pos, map, &mut self.clients).await;
            }
        }
    }

    /// Drains the command queue accumulated during opcode/GM handling and
    /// performs the corresponding world mutation + AOI broadcast. This is the
    /// single place that spawns/kills/sim-instantiates — handlers themselves
    /// must not touch `self.creatures` / `self.simulated_players` directly.
    #[tracing::instrument(level = "info", skip_all, name = "apply_commands")]
    async fn apply_commands(&mut self, queue: &mut crate::world::command::CommandQueue) {
        use crate::world::command::WorldCommand;
        for cmd in queue.drain() {
            match cmd {
                WorldCommand::SpawnCreature(mut creature) => {
                    let map = creature.map;
                    // Snap to terrain at spawn; also re-seat the wander anchor
                    // so future target picks inherit the corrected z. No-op
                    // when pathfinding isn't configured for this map.
                    if let Some(z) = self.maps.ground_height(
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
                    let create_object = creature.to_create_object();
                    let key = self.creatures.insert(creature);
                    self.register_creature(key);
                    if let Some(msg) = UpdateObject::from_objects(vec![create_object]) {
                        msg.broadcast_within_aoi(pos, map, &mut self.clients).await;
                    }
                }
                WorldCommand::KillCreature(kill_guid) => {
                    let Some(creature_key) = self.creature_by_guid.get(&kill_guid).copied()
                    else {
                        continue;
                    };
                    self.kill_creature(creature_key).await;
                }
                WorldCommand::SpawnSimulant(mut sim) => {
                    let map = sim.map;
                    if let Some(z) = self.maps.ground_height(
                        map,
                        sim.info.position.x,
                        sim.info.position.y,
                        sim.info.position.z,
                    ) {
                        sim.info.position.z = z;
                    }
                    let pos = sim.info.position;
                    let guid = sim.guid;
                    let create_object = simulated_create_object(&sim);
                    let start_msg = MSG_MOVE_START_FORWARD_Server {
                        guid,
                        info: sim.info.clone(),
                    };
                    let key = self.simulated_players.insert(sim);
                    self.simulated_by_guid.insert(guid, key);
                    if let Some(msg) = UpdateObject::from_objects(vec![create_object]) {
                        msg.broadcast_within_aoi(pos, map, &mut self.clients).await;
                    }
                    aoi::broadcast_within_aoi(start_msg, pos, map, &mut self.clients).await;
                }
            }
        }
    }

    // Declarations of `t_*` Duration variables get assigned exactly once
    // inside their respective phase blocks below. Clippy flags the late init
    // as redundant — but consolidating the per-phase timing handles at the
    // top makes the slow-tick log line trivially scannable.
    #[allow(clippy::needless_late_init)]
    #[tracing::instrument(level = "info", skip_all, name = "World::tick")]
    pub async fn tick(&mut self, db: &mut WorldDatabase, slow_warn: Duration) {
        let tick_start = Instant::now();
        // Wall-clock seconds since the last tick started. Clamped to 1 s so a
        // briefly frozen tick doesn't blow the auto-attack timer negative.
        let tick_dt: f32 = self
            .last_tick_at
            .map(|t| {
                let d = tick_start.duration_since(t).as_secs_f32();
                d.min(1.0)
            })
            .unwrap_or(crate::world::TARGET_INTERVAL.as_secs_f32());
        self.last_tick_at = Some(tick_start);

        // Per-phase timing accumulators. If the whole tick is slow we dump
        // these as a single WARN at the end so the operator can read the
        // breakdown without attaching Tracy.
        let t_drain: Duration;
        let t_chrscreen: Duration;
        let t_promote: Duration;
        let t_per_client: Duration;
        let t_apply_cmds: Duration;
        let t_corpses: Duration;
        let t_creatures: Duration;
        let t_sims: Duration;
        let t_logouts: Duration;

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
            for client in &mut self.clients_on_character_screen {
                handle_character_screen_opcodes(client, db).await;
            }
        }
        .instrument(tracing::info_span!("character_screen_opcodes"))
        .await;
        t_chrscreen = phase.elapsed();

        let phase = Instant::now();
        async {
        // Promoting a player builds an `UpdateObject` for every other client
        // visible from the new player's position. With N bots promoting in
        // one tick this would regenerate the same create-object N times per
        // already-in-world client. Build once per tick and reuse.
        let mut create_object_cache: ahash::AHashMap<Guid, Object> = ahash::AHashMap::new();
        while let Some(i) = self
            .clients_on_character_screen
            .iter()
            .position(|a| matches!(a.status, CharacterScreenProgress::WaitingToLogIn(_)))
        {
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

            let new_player_object = player_create_object(c.character());
            if let Some(msg) = UpdateObject::from_objects(vec![new_player_object]) {
                msg.broadcast_within_aoi(new_player_pos, new_player_map, &mut self.clients)
                    .await;
            }

            let mut visible_objects: Vec<Object> = Vec::new();
            let mut movement_starts: Vec<MSG_MOVE_START_FORWARD_Server> = Vec::new();

            for (_, client) in &self.clients {
                if client.character().map == new_player_map
                    && aoi::within_aoi(&client.character().info.position, &new_player_pos)
                {
                    let obj = create_object_cache
                        .entry(client.character().guid)
                        .or_insert_with(|| player_create_object(client.character()))
                        .clone();
                    visible_objects.push(obj);
                    // If this player is mid-motion, also queue a movement-start
                    // so the new client animates them instead of seeing a
                    // stationary object that teleports on every heartbeat.
                    if client.character().info.flags.get_forward() {
                        movement_starts.push(MSG_MOVE_START_FORWARD_Server {
                            guid: client.character().guid,
                            info: client.character().info.clone(),
                        });
                    }
                }
            }

            for (_, creature) in &self.creatures {
                if creature.map == new_player_map
                    && aoi::within_aoi(&creature.info.position, &new_player_pos)
                {
                    visible_objects.push(creature.to_create_object());
                    if creature.info.flags.get_forward() {
                        movement_starts.push(MSG_MOVE_START_FORWARD_Server {
                            guid: creature.guid,
                            info: creature.info.clone(),
                        });
                    }
                }
            }

            for (_, sim) in &self.simulated_players {
                if sim.map == new_player_map
                    && aoi::within_aoi(&sim.info.position, &new_player_pos)
                {
                    visible_objects.push(simulated_create_object(sim));
                    movement_starts.push(MSG_MOVE_START_FORWARD_Server {
                        guid: sim.guid,
                        info: sim.info.clone(),
                    });
                }
            }

            let visible_count = visible_objects.len();
            let starts_count = movement_starts.len();
            if let Some(batch) = UpdateObject::from_objects(visible_objects) {
                batch.send(&mut c).await;
            }
            for start in movement_starts {
                c.send_message(start).await;
            }
            tracing::debug!(
                "promote: account={} name={} pos=({:.1},{:.1},{:.1}) map={:?} -> sent {} CreateObjects + {} MoveStarts; clients_in_world={} creatures={} sims={}",
                c.session.account_name,
                c.character().name,
                new_player_pos.x,
                new_player_pos.y,
                new_player_pos.z,
                new_player_map,
                visible_count,
                starts_count,
                self.clients.len(),
                self.creatures.len(),
                self.simulated_players.len(),
            );

            self.clients.insert(c);
        }
        }
        .instrument(tracing::info_span!("promote_logged_in"))
        .await;
        t_promote = phase.elapsed();

        let mut keys_to_move_to_character_screen: Vec<usize> = Vec::new();
        let mut move_to_character_screen = false;
        let mut commands = crate::world::command::CommandQueue::new();

        let phase = Instant::now();
        async {
        let client_keys: Vec<usize> = self.clients.iter().map(|(k, _)| k).collect();
        for key in client_keys {
            let mut client = self.clients.remove(key);
            let mut entities = Entities::new(
                &mut self.clients,
                &mut self.creatures,
                &self.creature_by_guid,
                &mut self.simulated_players,
                &self.simulated_by_guid,
                &mut self.pending_movement,
            );
            world_opcode_handler::handle_received_client_opcodes(
                &mut client,
                &mut entities,
                db,
                &mut move_to_character_screen,
                &mut self.maps,
                &mut commands,
            )
            .await;
            client.character_mut().update_auto_attack_timer(tick_dt);

            if client.character().attacking && client.character().auto_attack_timer <= 0.0 {
                client.character_mut().auto_attack_timer = UNARMED_SPEED;
                const SWING_DAMAGE: u32 = 1332;
                let target_guid = client.character().target;
                let msg = SMSG_ATTACKERSTATEUPDATE {
                    hit_info: HitInfo::CriticalHit,
                    attacker: client.character().guid,
                    target: target_guid,
                    total_damage: SWING_DAMAGE,
                    damages: vec![DamageInfo {
                        spell_school_mask: 0,
                        damage_float: SWING_DAMAGE as f32,
                        damage_uint: SWING_DAMAGE,
                        absorb: 0,
                        resist: 0,
                    }],
                    unknown1: 0,
                    spell_id: 0,
                    damage_state: 0,
                    blocked_amount: 0,
                };

                let attacker_pos = client.character().info.position;
                let attacker_map = client.character().map;
                // Source is held outside the slab (removed for processing),
                // so `broadcast_within_aoi` only reaches other observers —
                // send to attacker separately. The broadcast helper
                // serializes once and reuses the body for all recipients.
                client.send_message(msg.clone()).await;
                aoi::broadcast_within_aoi(msg, attacker_pos, attacker_map, &mut self.clients)
                    .await;

                let target_key = self.creature_by_guid.get(&target_guid).copied();
                if let Some(creature_key) = target_key {
                    let creature = &mut self.creatures[creature_key];
                    creature.health = creature.health.saturating_sub(SWING_DAMAGE);
                    let creature_map = creature.map;
                    let creature_pos = creature.info.position;
                    let creature_guid = creature.guid;
                    let killed = creature.health == 0;

                    if killed {
                        self.kill_creature(creature_key).await;
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
                            &mut self.clients,
                        )
                        .await;
                    }
                }
            }

            if move_to_character_screen {
                keys_to_move_to_character_screen.push(key);
            }

            let new_key = self.clients.insert(client);
            debug_assert_eq!(new_key, key);
        }
        }
        .instrument(tracing::info_span!("per_client_loop"))
        .await;
        t_per_client = phase.elapsed();

        // Flush coalesced movement broadcasts. Each entry was queued by a
        // movement opcode handler this tick; we issue at most one broadcast
        // per source per tick via the serialize-once `broadcast_opcode_within_aoi`
        // path. The map is reused across ticks — `.drain()` keeps capacity.
        //
        // Crucially we pass `Some(source_guid)` so the source player does NOT
        // receive their own movement opcode back: at this point the source is
        // back in `self.clients` (per_client_loop re-inserted them), and any
        // echo would be treated by the local client as a server position
        // correction — visible as rubber-band / "laggy movement" on the
        // player's own character.
        {
            let _s = tracing::info_span!("flush_movement_broadcasts").entered();
            for (source_guid, pm) in self.pending_movement.drain() {
                aoi::broadcast_opcode_within_aoi(
                    &pm.msg,
                    pm.anchor,
                    pm.map,
                    Some(source_guid),
                    &mut self.clients,
                );
            }
        }

        let phase = Instant::now();
        self.apply_commands(&mut commands).await;
        t_apply_cmds = phase.elapsed();

        let phase = Instant::now();
        self.tick_corpses_and_respawns().await;
        t_corpses = phase.elapsed();

        let phase = Instant::now();
        self.tick_creature_ai().await;
        t_creatures = phase.elapsed();

        let phase = Instant::now();
        self.tick_simulated_players().await;
        t_sims = phase.elapsed();

        let phase = Instant::now();
        async {
        for key in keys_to_move_to_character_screen {
            let c = self.clients.remove(key);
            let logout_pos = c.character().info.position;
            let logout_map = c.character().map;
            for (_, a) in &mut self.clients {
                if a.character().map == logout_map
                    && aoi::within_aoi(&a.character().info.position, &logout_pos)
                {
                    a.send_message(SMSG_DESTROY_OBJECT {
                        guid: c.character().guid,
                    })
                    .await;
                }
            }

            let c = c.into_character_screen_client();
            self.clients_on_character_screen.push(c);
        }
        }
        .instrument(tracing::info_span!("drain_logouts"))
        .await;
        t_logouts = phase.elapsed();

        let stale_client_keys: Vec<usize> = self
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
            let c = self.clients.remove(key);
            let logout_map = c.character().map;
            let guid = c.character().guid;
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
            let total = guids.len();
            let msg = SMSG_UPDATE_OBJECT {
                has_transport: 0,
                objects: vec![Object {
                    update_type: Object_UpdateType::OutOfRangeObjects { guids },
                }],
            };
            let mut delivered = 0_usize;
            for (_, c) in self.clients.iter_mut() {
                if c.character().map == map {
                    c.send_message(msg.clone()).await;
                    delivered += 1;
                }
            }
            tracing::debug!(
                "Despawned {total} stale clients on map {map:?} -> notified {delivered} observers"
            );
        }

        while let Some((i, _)) = self
            .clients_on_character_screen
            .iter()
            .enumerate()
            .find(|(_, a)| a.reader_handle.is_finished())
        {
            self.clients_on_character_screen.remove(i);
        }

        let now_packet_count =
            crate::world::world::client::outgoing_packet_count();
        let packet_delta = now_packet_count.saturating_sub(self.last_packet_sample);
        let elapsed_secs = self
            .last_packet_sample_at
            .elapsed()
            .as_secs_f64()
            .max(1e-6);
        let packets_per_second = packet_delta as f64 / elapsed_secs;
        self.last_packet_sample = now_packet_count;
        self.last_packet_sample_at = Instant::now();

        if let Some(client) = tracy_client::Client::running() {
            let wander = self.creature_wander_count;
            let waypoint = self.creature_waypoint_count;
            let aggro = self.aggro_creature_keys.len();
            let idle = self.creatures.len().saturating_sub(wander + waypoint + aggro);
            client.plot(
                tracy_client::plot_name!("players"),
                self.clients.len() as f64,
            );
            client.plot(
                tracy_client::plot_name!("creatures"),
                self.creatures.len() as f64,
            );
            client.plot(tracy_client::plot_name!("creatures_idle"), idle as f64);
            client.plot(tracy_client::plot_name!("creatures_wander"), wander as f64);
            client.plot(
                tracy_client::plot_name!("creatures_waypoint"),
                waypoint as f64,
            );
            client.plot(tracy_client::plot_name!("creatures_aggro"), aggro as f64);
            client.plot(
                tracy_client::plot_name!("simulated_players"),
                self.simulated_players.len() as f64,
            );
            client.plot(
                tracy_client::plot_name!("char_screen_clients"),
                self.clients_on_character_screen.len() as f64,
            );
            client.plot(
                tracy_client::plot_name!("tick_ms"),
                tick_start.elapsed().as_secs_f64() * 1000.0,
            );
            client.plot(
                tracy_client::plot_name!("packets_per_second"),
                packets_per_second,
            );
            client.plot(
                tracy_client::plot_name!("adt_tiles_loaded"),
                self.maps.attempted_adt_count() as f64,
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
            tracing::warn!(
                target: "tick_slow",
                "slow tick total={:.1}ms drain={:.1} chrscreen={:.1} promote={:.1} per_client={:.1} apply={:.1} corpses={:.1} creatures={:.1} sims={:.1} logouts={:.1} | clients={} sims_n={} creatures_active={}",
                ms(total),
                ms(t_drain),
                ms(t_chrscreen),
                ms(t_promote),
                ms(t_per_client),
                ms(t_apply_cmds),
                ms(t_corpses),
                ms(t_creatures),
                ms(t_sims),
                ms(t_logouts),
                self.clients.len(),
                self.simulated_players.len(),
                self.walking_creature_keys.len(),
            );
        }
    }

    #[tracing::instrument(level = "info", skip_all, name = "tick_creature_ai")]
    async fn tick_creature_ai(&mut self) {
        const RE_PATH_THRESHOLD: f32 = 0.5;
        const STAND_OFF: f32 = 3.0;
        const MAX_FOLLOW_RANGE: f32 = 60.0;

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

        use rayon::prelude::*;
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
                            <= MAX_FOLLOW_RANGE * MAX_FOLLOW_RANGE
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
                let target_x = player_pos.x + STAND_OFF * angle.cos();
                let target_y = player_pos.y + STAND_OFF * angle.sin();

                let dx = target_x - from.x;
                let dy = target_y - from.y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < RE_PATH_THRESHOLD {
                    continue;
                }

                let to = Vector3d {
                    x: target_x,
                    y: target_y,
                    z: player_pos.z,
                };
                // Rust 1.45+: float-to-int `as` casts saturate; NaN -> 0, +inf -> u32::MAX.
                // `.max(0.0)` guards a negative input (which `as u32` would also yield 0).
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

            for (_, c) in &mut self.clients {
                if c.character().map == map
                    && aoi::within_aoi(&c.character().info.position, &to)
                {
                    c.send_message(msg.clone()).await;
                }
            }
        }

        self.tick_walking_creatures(now).await;
    }

    #[tracing::instrument(level = "info", skip_all, name = "tick_walking_creatures")]
    async fn tick_walking_creatures(&mut self, now: Instant) {
        const HEARTBEAT_INTERVAL_MS: u128 = 500;
        const ARRIVAL_THRESHOLD: f32 = 0.4;
        const WANDER_IDLE_MIN_MS: u64 = 3000;
        const WANDER_IDLE_MAX_MS: u64 = 8000;

        // Wake up any parked creatures whose idle window has elapsed.
        // BTreeMap is sorted ascending so we can stop at the first non-expired
        // entry. Stale keys (creatures killed while parked) get filtered here.
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

        // AOI gate: compute one bounding box per map covering every client's
        // AOI radius. Creatures whose position falls outside their map's box
        // skip the body entirely — no AI tick, no broadcast, nothing. When a
        // player walks back into range, the creature picks up wherever it was
        // (paused mid-route is fine because nobody saw it). With 51 k worlddb
        // creatures and clients clustered at the Gurubashi Arena spawn this
        // prunes ~99% of the per-tick work. Scratch map is reused across ticks.
        self.scratch_client_aabb.clear();
        for (_, cl) in self.clients.iter() {
            let p = cl.character().info.position;
            let map = cl.character().map;
            let entry = self
                .scratch_client_aabb
                .entry(map)
                .or_insert((f32::MAX, f32::MAX, f32::MIN, f32::MIN));
            entry.0 = entry.0.min(p.x - aoi::AOI_RADIUS_YARDS);
            entry.1 = entry.1.min(p.y - aoi::AOI_RADIUS_YARDS);
            entry.2 = entry.2.max(p.x + aoi::AOI_RADIUS_YARDS);
            entry.3 = entry.3.max(p.y + aoi::AOI_RADIUS_YARDS);
        }

        // Per-tick scratch lists held on `self` so the underlying allocations
        // persist tick-over-tick. Take by `mem::take` so the loop body has
        // disjoint `&mut self.creatures` access; restored at end.
        let mut events = std::mem::take(&mut self.scratch_walk_events);
        events.clear();
        let mut to_park = std::mem::take(&mut self.scratch_to_park);
        to_park.clear();
        let client_aabb = std::mem::take(&mut self.scratch_client_aabb);

        // Temporarily move walking_creature_keys out so the loop can take a
        // disjoint &mut on self.creatures; restored at end.
        let walking_keys = std::mem::take(&mut self.walking_creature_keys);
        for &key in &walking_keys {
            let c = &mut self.creatures[key];
            // Skip when no client is within AOI of this creature's map+position.
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
            // wow_world_base::DEFAULT_WALKING_SPEED is 1.0 yd/s — that's
            // ~40% of the canonical vanilla walking pace and looks crawly
            // in-client. 2.5 matches what the retail walking animation expects.
            let step = WALK_SPEED * dt;
            let map = c.map;

            // Phase A: behavior-specific entry transitions (idle -> moving).
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
                            // Park until decision time. Pulled out of the
                            // active iteration list so subsequent ticks don't
                            // pay for this creature at all.
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
                            // Park until idle expires.
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

            // Snapshot current target after potential mutation.
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

            // Phase B: step toward target.
            let dx = target.x - c.info.position.x;
            let dy = target.y - c.info.position.y;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist <= step || dist <= ARRIVAL_THRESHOLD {
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
                        let span = WANDER_IDLE_MAX_MS - WANDER_IDLE_MIN_MS;
                        let idle_ms = WANDER_IDLE_MIN_MS
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
                // Park on arrival too — saves one tick of iteration before
                // the early-continue idle branch would park anyway.
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
                        >= HEARTBEAT_INTERVAL_MS
                {
                    events.push((key, c.info.position, map, CreatureMoveEvent::Heartbeat));
                    c.last_heartbeat_at = now;
                }
            }
        }
        // Restore the active list, minus everything we parked this tick.
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
        // Park-out: stash `to_park`/`client_aabb` back; `events` is drained
        // by the broadcast loop below and then stored.
        self.scratch_to_park = to_park;
        self.scratch_client_aabb = client_aabb;

        // Snap each emitted event's position to ground if pathfinding maps
        // are available for the continent. Writes back into the creature so
        // its in-memory state stays consistent for future ticks. Use `.get`
        // since a future kill-during-tick rule could invalidate a key.
        for (key, _, map, _) in &events {
            let Some(creature) = self.creatures.get(*key) else {
                continue;
            };
            let xy = (creature.info.position.x, creature.info.position.y);
            let z_hint = creature.info.position.z;
            if let Some(z) = self.maps.ground_height(*map, xy.0, xy.1, z_hint)
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
        // Restore drained event buffer so the allocation persists.
        self.scratch_walk_events = events;
    }

    #[tracing::instrument(level = "info", skip_all, name = "tick_simulated_players")]
    async fn tick_simulated_players(&mut self) {
        const HEARTBEAT_INTERVAL_MS: u128 = 250;

        let now = Instant::now();

        // Phase 0: clear expired roots, restore FORWARD flag, broadcast aura-clear + start-forward.
        let mut expired_roots = std::mem::take(&mut self.scratch_expired_roots);
        expired_roots.clear();
        for (_, sim) in self.simulated_players.iter_mut() {
            if let Some(until) = sim.root_until
                && until <= now
            {
                sim.root_until = None;
                sim.info.flags = MovementInfo_MovementFlags::new_forward();
                expired_roots.push((sim.guid, sim.map, sim.info.position, sim.info.clone()));
            }
        }
        for (guid, map, pos, info) in expired_roots.drain(..) {
            let clear = SMSG_UPDATE_OBJECT {
                has_transport: 0,
                objects: vec![Object {
                    update_type: Object_UpdateType::Values {
                        guid1: guid,
                        mask1: UpdateMask::Player(
                            UpdatePlayerBuilder::new()
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
            let resume = MSG_MOVE_START_FORWARD_Server { guid, info };
            aoi::broadcast_within_aoi(resume, pos, map, &mut self.clients).await;
        }
        // Restore the drained Vec so its capacity survives to the next tick.
        self.scratch_expired_roots = expired_roots;

        // Phase 1: advance each puppet, decide which messages to emit.
        let mut events: Vec<(usize, Vector3d, Map, SimEvent)> = Vec::new();
        for (key, sim) in self.simulated_players.iter_mut() {
            if sim.current_wp >= sim.waypoints.len() {
                continue;
            }
            if sim.is_rooted() {
                sim.last_advanced_at = now;
                continue;
            }
            let dt = now
                .saturating_duration_since(sim.last_advanced_at)
                .as_secs_f32()
                .min(0.5);
            sim.last_advanced_at = now;
            let step = sim.movement_speed * dt;
            let target = sim.waypoints[sim.current_wp];
            let dx = target.x - sim.info.position.x;
            let dy = target.y - sim.info.position.y;
            let dist = (dx * dx + dy * dy).sqrt();

            let mut emit_facing = false;
            if dist <= step {
                sim.info.position = target;
                sim.current_wp += 1;
                emit_facing = true;
                if sim.current_wp >= sim.waypoints.len() {
                    events.push((key, sim.info.position, sim.map, SimEvent::Despawn));
                    continue;
                }
                let next = sim.waypoints[sim.current_wp];
                let ndx = next.x - sim.info.position.x;
                let ndy = next.y - sim.info.position.y;
                sim.info.orientation = ndy.atan2(ndx);
            } else {
                sim.info.position.x += step * dx / dist;
                sim.info.position.y += step * dy / dist;
                sim.info.position.z = target.z;
            }

            let should_heartbeat = now
                .saturating_duration_since(sim.last_heartbeat)
                .as_millis()
                >= HEARTBEAT_INTERVAL_MS;

            if now >= sim.next_jump_at {
                events.push((key, sim.info.position, sim.map, SimEvent::Jump));
                sim.next_jump_at = now
                    + std::time::Duration::from_millis(
                        5000 + crate::world::world_opcode_handler::gm_command::next_rand() % 10000,
                    );
                sim.last_heartbeat = now;
            } else if emit_facing {
                events.push((key, sim.info.position, sim.map, SimEvent::SetFacing));
                sim.last_heartbeat = now;
            } else if should_heartbeat {
                events.push((key, sim.info.position, sim.map, SimEvent::Heartbeat));
                sim.last_heartbeat = now;
            }
        }

        // Phase 1.5: snap to ground only on phase transitions (SetFacing,
        // Jump), not on every heartbeat. Each `ground_height` call is a
        // navmesh raycast — at 100 sims × 4 heartbeats/sec the per-tick cost
        // ran into 800ms-plus. Heartbeats now linearly interpolate Z; the
        // next phase transition corrects accumulated drift.
        for (key, _, map, event) in &events {
            if matches!(event, SimEvent::Heartbeat) {
                continue;
            }
            if let Some(sim) = self.simulated_players.get(*key) {
                let xy = (sim.info.position.x, sim.info.position.y);
                let z_hint = sim.info.position.z;
                if let Some(z) = self.maps.ground_height(*map, xy.0, xy.1, z_hint) {
                    self.simulated_players[*key].info.position.z = z;
                }
            }
        }

        // Phase 2: dispatch messages + remove despawned. Use `.get` so a
        // duplicate Despawn or any future remove-during-tick doesn't panic.
        for (key, _, map, event) in events {
            match event {
                SimEvent::Heartbeat => {
                    let Some(sim) = self.simulated_players.get(key) else {
                        continue;
                    };
                    let msg = MSG_MOVE_HEARTBEAT_Server {
                        guid: sim.guid,
                        info: sim.info.clone(),
                    };
                    let pos = sim.info.position;
                    aoi::broadcast_within_aoi(msg, pos, map, &mut self.clients).await;
                }
                SimEvent::SetFacing => {
                    let Some(sim) = self.simulated_players.get(key) else {
                        continue;
                    };
                    let msg = MSG_MOVE_SET_FACING_Server {
                        guid: sim.guid,
                        info: sim.info.clone(),
                    };
                    let pos = sim.info.position;
                    aoi::broadcast_within_aoi(msg, pos, map, &mut self.clients).await;
                }
                SimEvent::Jump => {
                    let Some(sim) = self.simulated_players.get(key) else {
                        continue;
                    };
                    let msg = MSG_MOVE_JUMP_Server {
                        guid: sim.guid,
                        info: sim.info.clone(),
                    };
                    let pos = sim.info.position;
                    aoi::broadcast_within_aoi(msg, pos, map, &mut self.clients).await;
                }
                SimEvent::Despawn => {
                    if !self.simulated_players.contains(key) {
                        continue;
                    }
                    let sim = self.simulated_players.remove(key);
                    self.simulated_by_guid.remove(&sim.guid);
                    let pos = sim.info.position;
                    let stop_info = MovementInfo {
                        flags: MovementInfo_MovementFlags::default(),
                        ..sim.info.clone()
                    };
                    let stop = MSG_MOVE_STOP_Server {
                        guid: sim.guid,
                        info: stop_info,
                    };
                    aoi::broadcast_within_aoi(stop, pos, map, &mut self.clients).await;
                    let destroy = SMSG_DESTROY_OBJECT { guid: sim.guid };
                    aoi::broadcast_within_aoi(destroy, pos, map, &mut self.clients).await;
                }
            }
        }
    }
}

enum SimEvent {
    Heartbeat,
    SetFacing,
    Jump,
    Despawn,
}

#[derive(Debug, Copy, Clone)]
enum CreatureMoveEvent {
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
    let mut obj = player_create_object(character);
    match &mut obj.update_type {
        Object_UpdateType::CreateObject2 { movement2, .. } => {
            movement2.update_flag = movement2.update_flag.clone().set_self();
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
                        walking_speed: WALK_SPEED,
                    },
                ),
            },
            object_type: ObjectType::Player,
        },
    }
}

fn get_update_object_player(character: &Character) -> UpdateMask {
    // Mirrors the field set used by `get_update_simulated_player_mask`,
    // which is known to render correctly to observers. The previous,
    // larger field list (stats, target, skill_info, XP, the unit_bytes
    // pair, combatreach, boundingradius) plus a `HIGH_GUID` movement
    // flag broke client-side rendering for OTHER players (admin logs in
    // near bots → bots invisible). The smaller mask is what works.
    //
    // Re-add anything from the larger set deliberately, one piece at a
    // time, with a real client test after each addition — the protocol
    // doesn't fail loud when a field is malformed, it just silently
    // discards or crashes the observer.
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
        .set_unit_base_health(character.max_health())
        .set_unit_health(character.max_health())
        .set_unit_maxhealth(character.max_health())
        .set_unit_level(character.level.as_int() as i32)
        .set_unit_factiontemplate(race.faction_id().as_int() as i32)
        .set_unit_displayid(race.display_id(character.gender))
        .set_unit_nativedisplayid(race.display_id(character.gender));

    // Visible-item slots only — `set_player_visible_item` carries the item
    // ENTRY + enchants, which is what the client needs to render gear on
    // the unit. We deliberately do NOT call `set_player_field_inv` here:
    // that field expects a properly-typed item GUID (`HIGHGUID_ITEM` =
    // 0x4000 in the high 32 bits), but our `db.new_guid()` hands out
    // type-less counters that look identical to player GUIDs on the wire.
    // Shipping those over `PLAYER_FIELD_INV_*` to an observer makes the
    // client interpret the slot as referring to a player guid, fails the
    // item lookup, and crashes on render. The puppet path
    // (`get_update_simulated_player_mask`) skips this for the same reason
    // and renders cleanly. Revisit when item guids get proper type bits.
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

    UpdateMask::Player(mask.finalize())
}

pub async fn announce_character_login(client: &mut Client, character: &Character) {
    if let Some(msg) = UpdateObject::from_objects(vec![player_create_object(character)]) {
        msg.send(client).await;
    }
}

pub fn simulated_create_object(p: &SimulatedPlayer) -> Object {
    let flags = if p.info.flags.get_forward() {
        MovementBlock_MovementFlags::new_forward()
    } else {
        MovementBlock_MovementFlags::empty()
    };
    Object {
        update_type: Object_UpdateType::CreateObject2 {
            guid3: p.guid,
            mask2: get_update_simulated_player_mask(p),
            movement2: MovementBlock {
                update_flag: MovementBlock_UpdateFlag::new_living(
                    MovementBlock_UpdateFlag_Living::Living {
                        backwards_running_speed: DEFAULT_RUNNING_BACKWARDS_SPEED,
                        backwards_swimming_speed: 0.0,
                        fall_time: 0.0,
                        flags,
                        living_orientation: p.info.orientation,
                        living_position: p.info.position,
                        running_speed: p.movement_speed,
                        swimming_speed: 0.0,
                        timestamp: 0,
                        turn_rate: DEFAULT_TURN_SPEED,
                        walking_speed: WALK_SPEED,
                    },
                ),
            },
            object_type: ObjectType::Player,
        },
    }
}

fn get_update_simulated_player_mask(p: &SimulatedPlayer) -> UpdateMask {
    let race = p.race_class.race();
    let class = p.race_class.class();
    let mut mask = UpdatePlayerBuilder::new()
        .set_object_guid(p.guid)
        .set_object_scale_x(race.race_scale(p.gender))
        .set_unit_bytes_0(race.into(), class, p.gender.into(), class.power_type())
        .set_player_bytes_2(p.facialhair, 0, 0, 2)
        .set_player_features(p.skin, p.face, p.hairstyle, p.haircolor)
        .set_unit_base_health(4000)
        .set_unit_health(4000)
        .set_unit_maxhealth(4000)
        .set_unit_level(p.level.as_int() as i32)
        .set_unit_factiontemplate(race.faction_id().as_int() as i32)
        .set_unit_displayid(race.display_id(p.gender))
        .set_unit_nativedisplayid(race.display_id(p.gender));

    for (i, entry) in p.equipment.iter().enumerate() {
        if let Some(entry) = entry
            && let Ok(index) = VisibleItemIndex::try_from(i)
            && let Some(item) = wow_items::vanilla::lookup_item(*entry)
        {
            let visible = VisibleItem::new(
                Guid::zero(),
                *entry,
                [0, 0],
                item.random_property() as u32,
                0,
            );
            mask = mask.set_player_visible_item(visible, index);
        }
    }

    UpdateMask::Player(mask.finalize())
}

pub fn get_client_login_messages(character: &Character) -> Vec<ServerOpcodeMessage> {
    let mut v = Vec::with_capacity(16);

    let year = 22;
    let month = 7;
    let month_day = 12;
    let week_day = 3;
    let hour = 8;
    let minute = 10;
    v.push(ServerOpcodeMessage::SMSG_LOGIN_SETTIMESPEED(
        SMSG_LOGIN_SETTIMESPEED {
            datetime: DateTime::new(
                year,
                month.try_into().unwrap(),
                month_day,
                week_day.try_into().unwrap(),
                hour,
                minute,
            ),
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
