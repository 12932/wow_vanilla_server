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
        self.creature_by_guid.shrink_to_fit();
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
    /// checks for each of N observers). At 1400 clients that's ~2M
    /// cheap distance checks per tick — fits in a few ms on a modern CPU,
    /// and is dwarfed by the actual broadcast IO. Spatial-grid bucketing
    /// is the next optimization if profiling shows this is hot.
    ///
    /// Only **players** are tracked here for now; creatures and simulated
    /// players use their own static-spawn / kill paths today. Extending
    /// to those is straightforward but out of scope for the despawn-on-
    /// AOI-exit fix this addresses.
    async fn tick_aoi_transitions(&mut self) {
        async {
        let client_keys: Vec<usize> = self.clients.iter().map(|(k, _)| k).collect();

        for key in client_keys {
            let mut observer = self.clients.remove(key);

            let observer_map = observer.character().map;
            let observer_pos = observer.character().info.position;

            // Build the new visible set from currently-connected clients
            // on the same map within AOI. `observer` is out of the slab
            // already, so the iteration naturally skips self.
            let mut new_visible: ahash::AHashSet<Guid> =
                ahash::AHashSet::with_capacity(observer.session.visible_entities.len());
            for (_, c) in self.clients.iter() {
                if c.character().map != observer_map {
                    continue;
                }
                if !aoi::within_aoi(&observer_pos, &c.character().info.position) {
                    continue;
                }
                new_visible.insert(c.character().guid);
            }

            // Take the old set, install the new one, diff for transitions.
            let old =
                std::mem::replace(&mut observer.session.visible_entities, new_visible.clone());
            let departed: Vec<Guid> = old.difference(&new_visible).copied().collect();
            let entered: Vec<Guid> = new_visible.difference(&old).copied().collect();

            // Despawn batch — one packet per observer regardless of how
            // many entities just left their AOI.
            if !departed.is_empty() {
                let msg = SMSG_UPDATE_OBJECT {
                    has_transport: 0,
                    objects: vec![Object {
                        update_type: Object_UpdateType::OutOfRangeObjects { guids: departed },
                    }],
                };
                observer.send_message(msg).await;
            }

            // Spawn batch — build a `CreateObject2` for each newcomer.
            // Inner lookup is a linear scan over `self.clients`; entered
            // is usually tiny (0-3 per tick) so we don't bother with a
            // reverse index. If profiling later shows this is hot, add a
            // `guid -> slab_key` map alongside `creature_by_guid`.
            if !entered.is_empty() {
                let mut objects: Vec<Object> = Vec::with_capacity(entered.len());
                for g in &entered {
                    for (_, c) in self.clients.iter() {
                        if c.character().guid == *g {
                            objects.push(player_create_object(c.character()));
                            break;
                        }
                    }
                }
                if let Some(msg) = UpdateObject::from_objects(objects) {
                    msg.send(&mut observer).await;
                }
            }

            let new_key = self.clients.insert(observer);
            debug_assert_eq!(new_key, key);
        }
        }
        .instrument(tracing::info_span!("tick_aoi_transitions"))
        .await;
    }

    async fn tick_corpses_and_respawns(&mut self) {
        let now = Instant::now();

        // Decide transitions without holding a mutable borrow on the slab.
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
            .unwrap_or(
                crate::config::config()
                    .tick
                    .target_interval()
                    .as_secs_f32(),
            );
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

            let new_player_guid = c.character().guid;
            let new_player_object = player_create_object(c.character());
            if let Some(msg) = UpdateObject::from_objects(vec![new_player_object]) {
                msg.broadcast_within_aoi(new_player_pos, new_player_map, &mut self.clients)
                    .await;
                // Seed every AOI observer's visible-entity set with the new
                // player so the upcoming AOI-transition pass doesn't re-emit
                // a duplicate `CreateObject` for them next tick. Mirrors the
                // broadcast distance check above — same map + same horizontal
                // radius.
                for (_, observer) in self.clients.iter_mut() {
                    if observer.character().map == new_player_map
                        && aoi::within_aoi(
                            &observer.character().info.position,
                            &new_player_pos,
                        )
                    {
                        observer.session.visible_entities.insert(new_player_guid);
                    }
                }
            }

            let mut visible_objects: Vec<Object> = Vec::new();
            let mut movement_starts: Vec<MSG_MOVE_START_FORWARD_Server> = Vec::new();

            for (_, client) in &self.clients {
                if client.character().map == new_player_map
                    && aoi::within_aoi(&client.character().info.position, &new_player_pos)
                {
                    let other_guid = client.character().guid;
                    let obj = create_object_cache
                        .entry(other_guid)
                        .or_insert_with(|| player_create_object(client.character()))
                        .clone();
                    visible_objects.push(obj);
                    // Same seeding logic for the new client's own visible set
                    // — they've now been told about this existing observer,
                    // so the AOI-transition pass shouldn't redundantly send
                    // it again on the next tick.
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

            let visible_count = visible_objects.len();
            let starts_count = movement_starts.len();
            if let Some(batch) = UpdateObject::from_objects(visible_objects) {
                batch.send(&mut c).await;
            }
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
                self.clients.len(),
                self.creatures.len(),
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

        // No server-side respawn pass — under the gurubashi-pvp rules
        // players stay dead where they fell. `time_of_death` remains set
        // and `is_dead()` keeps the opcode handler dropping incoming
        // packets, so a dead client sits inertly as a corpse until the
        // server restarts (snapshot load resets `current_health` to
        // `max_health`).

        let phase = Instant::now();
        async {
        let client_keys: Vec<usize> = self.clients.iter().map(|(k, _)| k).collect();
        for key in client_keys {
            let mut client = self.clients.remove(key);
            let mut entities = Entities::new(
                &mut self.clients,
                &mut self.creatures,
                &self.creature_by_guid,
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
                } else if let Some(&ck) = self.creature_by_guid.get(&target_guid) {
                    let cr = &self.creatures[ck];
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
                    self.clients
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
                    let new_key = self.clients.insert(client);
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
                        &mut self.clients,
                    )
                    .await;

                    match kind {
                        SwingKind::Creature(creature_key) => {
                            let creature = &mut self.creatures[creature_key];
                            creature.health = creature.health.saturating_sub(swing_damage);
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
                        SwingKind::Player(target_key) => {
                            let target = &mut self.clients[target_key];
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
                            // Target is in `self.clients` so they'll receive via
                            // broadcast; attacker is held outside, send directly.
                            client.send_message(hp_update.clone()).await;
                            aoi::broadcast_within_aoi(
                                hp_update,
                                target_pos,
                                target_map,
                                &mut self.clients,
                            )
                            .await;
                        }
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
            for (source_guid, pm) in self.pending_movement.drain() {
                let (r, b) = aoi::broadcast_opcode_within_aoi(
                    &pm.msg,
                    pm.anchor,
                    pm.map,
                    Some(source_guid),
                    &mut self.clients,
                );
                sources += 1;
                recipients += r;
                bytes += r * b;
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
            }
        }

        // AOI transitions: for each connected player, diff their previously
        // visible set against the players currently within `AOI_RADIUS_YARDS`
        // on the same map. Anything that left → `OutOfRangeObjects`
        // (despawn). Anything that entered → `CreateObject2` (spawn).
        // Without this pass, players who walk past the AOI boundary
        // linger forever on observers' clients as motionless ghosts.
        self.tick_aoi_transitions().await;

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
        async {
        for key in keys_to_move_to_character_screen {
            let c = self.clients.remove(key);
            let logout_pos = c.character().info.position;
            let logout_map = c.character().map;
            let logout_guid = c.character().guid;
            for (_, a) in &mut self.clients {
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
            for (_, c) in self.clients.iter_mut() {
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
                "slow tick total={:.1}ms drain={:.1} chrscreen={:.1} promote={:.1} per_client={:.1} apply={:.1} corpses={:.1} creatures={:.1} logouts={:.1} | clients={} creatures_active={}",
                ms(total),
                ms(t_drain),
                ms(t_chrscreen),
                ms(t_promote),
                ms(t_per_client),
                ms(t_apply_cmds),
                ms(t_corpses),
                ms(t_creatures),
                ms(t_logouts),
                self.clients.len(),
                self.walking_creature_keys.len(),
            );
        }
    }

    #[tracing::instrument(level = "info", skip_all, name = "tick_creature_ai")]
    async fn tick_creature_ai(&mut self) {
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
        let creature_cfg = &crate::config::config().creature;
        let heartbeat_interval_ms = creature_cfg.walking_heartbeat_ms;
        let arrival_threshold = creature_cfg.arrival_threshold;
        let wander_idle_min_ms = creature_cfg.wander_idle_min_ms;
        let wander_idle_max_ms = creature_cfg.wander_idle_max_ms;

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
            let step = walk_speed() * dt;
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
                        >= heartbeat_interval_ms
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
                        walking_speed: walk_speed(),
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
