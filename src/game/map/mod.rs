//! Maps: each map, or instance of one, is an actor that owns everything in
//! it, handles its players' packets as they arrive, and runs a 20 Hz update
//! for what time drives.

pub mod account_data;
pub mod ai;
pub mod auction;
pub mod arena;
pub mod assistance;
pub mod auras;
pub mod battleground;
pub mod boss_mechanics;
pub mod chat;
pub mod cinematic;
pub mod combat;
pub mod conditions;
pub mod corpses;
pub mod creature;
pub mod creature_groups;
pub mod creature_motion;
pub mod creature_combat;

pub mod death;
pub mod derive;
pub mod duel;
pub mod enchant;
pub mod environment;
pub mod eviction;
pub mod exploration;
pub mod event_ai;
pub mod escorts;
pub mod extra_effects;
pub mod gameobject;
pub mod gm;
pub mod gm_dev;
pub mod grid;
pub mod guard_posts;
pub mod handlers;
mod instance_adapters;
pub mod instance_script;
pub mod intoxication;
pub mod inventory;
pub mod item_scripts;
pub mod item_rules;
pub mod items_misc;
pub mod items;
pub mod local_defense;
pub mod login;
pub mod loot;
pub mod meeting_stones;
pub mod misc;
pub mod map_events;
pub mod mounts;
pub mod movement;
pub mod nav;
pub mod npc;
pub mod objects;
pub mod outdoor_pvp;
pub mod pets;
pub mod player;
pub mod player_motion;
pub mod professions;
pub mod proc_scripts;
pub mod procs;
pub mod pvp;
pub mod query;
pub mod quests;
pub mod ranged;
pub mod questshare;
pub mod reputation;
pub mod rest;
pub mod scripts;
pub mod skills;
pub mod social;
pub mod spawn_auras;
pub mod spell_areas;
pub mod spells;
pub mod stats;
pub mod terrain;
pub mod tickets;
pub mod trade;
pub mod trade_spells;
pub mod trinkets;
pub mod vendor;
pub mod visibility;
pub mod world;
pub mod world_states;

use std::sync::Arc;
use std::time::{Duration, Instant};

use ahash::AHashMap;
use parking_lot::RwLock;
use slab::Slab;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;

use crate::game::Realm;
use crate::game::fields::Visibility;
use crate::game::guid;
use crate::game::map::creature::Creature;
use crate::game::map::gameobject::GameObject;
use crate::game::map::grid::Grid;
use crate::game::map::player::Player;
use crate::game::registry::Online;
use crate::game::session::Control;
use crate::game::store::CharacterRow;
use crate::game::update::{self, Batch};

/// A map, and which copy of it: 0 for the open world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MapKey {
    pub map: u32,
    pub instance: u32,
}

impl MapKey {
    pub const fn world(map: u32) -> Self {
        Self { map, instance: 0 }
    }
}

impl std::fmt::Display for MapKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.instance == 0 {
            write!(f, "map {}", self.map)
        } else {
            write!(f, "map {} instance {}", self.map, self.instance)
        }
    }
}

/// What a map is sent.
#[allow(clippy::large_enum_variant)] // Packets move through the channel once.
pub enum MapEvent {
    Arrive(Box<Arrival>),
    Packet(Guid, ClientOpcodeMessage),
    Disconnect(Guid),
    /// Run code on the map: how a service changes something a map owns.
    Call(Box<dyn FnOnce(&mut Map) + Send>),
    /// Save everyone and stop.
    Shutdown(oneshot::Sender<()>),
    /// Stop: an empty copy that goes.
    Stop,
}

/// A player coming into a map.
pub struct Arrival {
    pub kind: ArrivalKind,
    pub online: Arc<Online>,
}

pub enum ArrivalKind {
    /// Logging in, from a stored character.
    Login(Box<CharacterRow>),
    /// From another map.
    Transfer(Box<Player>),
}

/// A handle to a map's actor.
#[derive(Clone)]
pub struct MapHandle {
    pub key: MapKey,
    tx: mpsc::UnboundedSender<MapEvent>,
}

impl std::fmt::Debug for MapHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapHandle({})", self.key)
    }
}

impl MapHandle {
    pub fn send(&self, event: MapEvent) -> bool {
        self.tx.send(event).is_ok()
    }

    pub fn call(&self, f: impl FnOnce(&mut Map) + Send + 'static) -> bool {
        self.send(MapEvent::Call(Box::new(f)))
    }

    /// Run `f` on the map and wait for what it returns.
    pub async fn query<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Map) -> T + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = oneshot::channel();
        self.call(move |m| {
            let _ = tx.send(f(m));
        });
        rx.await.ok()
    }
}

/// Every map that is running.
#[derive(Default)]
pub struct Maps {
    handles: RwLock<AHashMap<MapKey, MapHandle>>,
}

impl Maps {
    /// The map's handle, starting its actor if it isn't running.
    pub fn get(&self, realm: &Arc<Realm>, key: MapKey) -> MapHandle {
        if let Some(h) = self.handles.read().get(&key) {
            return h.clone();
        }
        let mut handles = self.handles.write();
        if let Some(h) = handles.get(&key) {
            return h.clone();
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = MapHandle { key, tx };
        let map = Map::new(key, realm.clone(), handle.clone());
        tokio::spawn(run(map, rx));
        handles.insert(key, handle.clone());
        info!("started {key}");
        handle
    }

    /// Stop a copy and forget it: the next visit starts a fresh one.
    pub fn stop(&self, key: MapKey) {
        if let Some(h) = self.handles.write().remove(&key) {
            h.send(MapEvent::Stop);
            info!("stopped {key}");
        }
    }

    pub fn all(&self) -> Vec<MapHandle> {
        self.handles.read().values().cloned().collect()
    }
}

/// How often each map logs its numbers.
pub const REPORT_EVERY: Duration = Duration::from_secs(60);

/// The map update's period.
pub const UPDATE_INTERVAL: Duration = Duration::from_millis(50);
/// How long a client has to acknowledge a teleport before its movement is
/// heeded again.
const TELEPORT_TIMEOUT: Duration = Duration::from_secs(5);

/// Per-map counters, for `.maps` and the logs.
#[derive(Debug, Default, Clone, Copy)]
pub struct MapStats {
    pub updates: u64,
    pub events: u64,
    pub last_update: Duration,
    pub max_update: Duration,
    /// Since the last report: updates, their total time, and the slowest.
    pub window_updates: u32,
    pub window_total: Duration,
    pub window_max: Duration,
    /// Events and updates that panicked and were let go.
    pub panics: u64,
}

pub struct Map {
    pub key: MapKey,
    pub realm: Arc<Realm>,
    pub handle: MapHandle,
    pub players: AHashMap<Guid, Player>,
    pub creatures: Slab<Creature>,
    pub creature_by_guid: AHashMap<Guid, usize>,
    pub gameobjects: Slab<GameObject>,
    pub gameobject_by_guid: AHashMap<Guid, usize>,
    pub corpses: AHashMap<Guid, death::Corpse>,
    /// Creatures dead and waiting to decay or respawn.
    pub dead_creatures: Vec<Guid>,
    /// Vendors' limited stock: (vendor, item).
    pub vendor_stock: AHashMap<(Guid, u32), vendor::Stock>,
    pub nav: Option<nav::Nav>,
    /// The map's terrain tiles: liquid kinds and areas.
    pub terrain: Option<terrain::Terrain>,
    pub grid: Grid,
    /// Objects whose visibility must be worked out at the next update.
    pub visibility_dirty: ahash::AHashSet<Guid>,
    /// Objects each observer must be told to forget at the next update.
    pub forget: AHashMap<Guid, Vec<Guid>>,
    /// Each observer's blocks for the coming update.
    pub batches: AHashMap<Guid, Batch>,
    /// Create blocks built this update, by object, for every observer that
    /// needs one.
    create_cache: AHashMap<Guid, Arc<[u8]>>,
    pub stats: MapStats,
    pub started: Instant,
    pub now: Instant,
    pub next_creature_counter: u32,
    pub next_spline_id: u32,
    /// When grouped players' stats next go out.
    pub member_stats_at: Instant,
    /// Since when a copy has had no one in it.
    pub empty_since: Option<Instant>,
    /// The actor stops after this update.
    pub stopping: bool,
    /// The match, on a battleground's copy.
    pub battleground: Option<battleground::Match>,
    /// Objects every player in the map knows: the boats and zeppelins.
    pub globals: Vec<Guid>,
    /// Rolls under way and gathered nodes to bring back.
    pub loot_state: loot::MapLoot,
    /// A continent's world PvP: the Plaguelands' towers, Silithus.
    pub outdoor: Option<outdoor_pvp::Outdoor>,
    /// The DB scripts running.
    pub scripts: scripts::Runner,
    /// The encounters' spell mechanics' own state.
    pub bosses: boss_mechanics::State,
    /// A copy's instance script and its data.
    pub instance: instance_script::State,
    /// Creatures whose spawned events run at the next update.
    pub ai_spawned: Vec<Guid>,
    /// Creatures by the spawn they came from.
    pub by_spawn: AHashMap<u32, Guid>,
    /// Objects a script changed that go back: (when, object, state; MAX
    /// to go).
    pub object_resets: Vec<(Instant, Guid, u32)>,
    /// Groups scripts made: member → (leader, distance, angle, flags).
    pub script_groups: AHashMap<Guid, (Guid, f32, f32, u32)>,
    /// Scripted map events under way.
    pub map_events: map_events::Events,
    pub spells: spells::MapSpells,
    /// Escort and follower quests under way.
    pub escorts: Vec<escorts::Active>,
    /// The Elemental Invasion's rifts and their invaders.
    pub rifts: AHashMap<Guid, crate::game::elemental::Rift>,
    pub rifts_checked_at: Option<Instant>,
    /// Gurubashi Arena's battles and Booty Run (on its continent).
    pub arena: arena::Arena,
    /// Objects spells summoned, traps, rituals and quest objects.
    pub objects: objects::Objects,
    /// Frames for after this update's create blocks: (observer, frame).
    pub late_frames: Vec<(Guid, Arc<[u8]>)>,
}

/// Run a map's actor until it is told to stop.
/// Run `f` on the map; a panic in it is logged and the map goes on, so a
/// bad packet or a bug costs one event or one update, not the map.
fn guarded<T>(map: &mut Map, f: impl FnOnce(&mut Map) -> T) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(map))) {
        Ok(v) => Some(v),
        Err(e) => {
            let what = e
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            tracing::error!("{} panicked: {what}; going on", map.key);
            map.stats.panics += 1;
            None
        }
    }
}

async fn run(mut map: Map, mut rx: mpsc::UnboundedReceiver<MapEvent>) {
    world::populate(&mut map);
    let mut report_at = Instant::now() + REPORT_EVERY;
    let mut ticker = tokio::time::interval(UPDATE_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            event = rx.recv() => {
                let Some(event) = event else { break };
                map.now = Instant::now();
                match guarded(&mut map, |m| m.handle_event(event)) {
                    Some(true) => break,
                    Some(false) => {}
                    None => continue,
                }
                // Drain what else has arrived, a bounded amount, so the
                // update isn't starved.
                for _ in 0..512 {
                    match rx.try_recv() {
                        Ok(event) => {
                            if guarded(&mut map, |m| m.handle_event(event)) == Some(true) {
                                return;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
            _ = ticker.tick() => {
                guarded(&mut map, Map::update);
                if map.stopping {
                    break;
                }
                if map.now >= report_at {
                    report_at = map.now + REPORT_EVERY;
                    map.report(rx.len());
                }
            }
        }
    }
}

impl Map {
    pub fn new(key: MapKey, realm: Arc<Realm>, handle: MapHandle) -> Self {
        let now = Instant::now();
        let instance = instance_script::State::for_map(&realm.data, key.map, key.instance);
        Self {
            key,
            realm,
            handle,
            players: AHashMap::new(),
            creatures: Slab::new(),
            creature_by_guid: AHashMap::new(),
            gameobjects: Slab::new(),
            gameobject_by_guid: AHashMap::new(),
            corpses: AHashMap::new(),
            dead_creatures: Vec::new(),
            vendor_stock: AHashMap::new(),
            nav: None,
            terrain: None,
            grid: Grid::new(),
            visibility_dirty: ahash::AHashSet::new(),
            forget: AHashMap::new(),
            batches: AHashMap::new(),
            create_cache: AHashMap::new(),
            stats: MapStats::default(),
            started: now,
            now,
            next_creature_counter: 1,
            next_spline_id: 1,
            member_stats_at: Instant::now(),
            empty_since: None,
            stopping: false,
            battleground: battleground::for_map(key, Instant::now()),
            globals: Vec::new(),
            loot_state: Default::default(),
            outdoor: outdoor_pvp::for_map(key),
            scripts: Default::default(),
            bosses: Default::default(),
            instance,
            ai_spawned: Vec::new(),
            by_spawn: AHashMap::new(),
            object_resets: Vec::new(),
            script_groups: AHashMap::new(),
            map_events: Default::default(),
            spells: Default::default(),
            escorts: Vec::new(),
            rifts: AHashMap::new(),
            rifts_checked_at: None,
            arena: Default::default(),
            objects: Default::default(),
            late_frames: Vec::new(),
        }
    }

    /// Milliseconds since the map started: the server's clock in movement
    /// packets.
    pub fn server_ms(&self) -> u32 {
        self.now.duration_since(self.started).as_millis() as u32
    }

    /// Handle one event; true to stop the actor.
    fn handle_event(&mut self, event: MapEvent) -> bool {
        self.stats.events += 1;
        match event {
            MapEvent::Arrive(a) => self.arrive(*a),
            MapEvent::Packet(guid, msg) => self.handle_packet(guid, msg),
            MapEvent::Disconnect(guid) => {
                if let Some(p) = self.take_player(guid) {
                    debug!("{} disconnected from {}", p.name, self.key);
                    self.save_and_go_offline(p);
                }
            }
            MapEvent::Call(f) => f(self),
            MapEvent::Stop => return true,
            MapEvent::Shutdown(reply) => {
                let guids: Vec<Guid> = self.players.keys().copied().collect();
                for g in guids {
                    if let Some(p) = self.take_player(g) {
                        self.save_and_go_offline(p);
                    }
                }
                let _ = reply.send(());
                return true;
            }
        }
        false
    }

    fn arrive(&mut self, arrival: Arrival) {
        let Arrival { kind, online } = arrival;
        online.set_map(self.handle.clone());
        match kind {
            ArrivalKind::Login(row) => {
                let mut p = Player::from_row(&self.realm.data.clone(), *row, online, self.now);
                p.row.map = self.key.map;
                let guid = p.guid;
                items::on_login(&mut p, &self.realm.data.clone());
                crate::game::transports::place_aboard(&self.realm, &mut p);
                login::send_login(self, &mut p);
                crate::game::instances::send_saved(&self.realm, p.guid, &p.outbound);
                let corpse = p.row.state.corpse;
                if p.row.state.dead {
                    match corpse {
                        Some(c) => {
                            death::become_ghost(&mut p, self.now);
                            let cg = guid::corpse(guid::counter(guid));
                            if c.map == self.key.map && !self.corpses.contains_key(&cg) {
                                let at = wow_world_messages::vanilla::Vector3d {
                                    x: c.x,
                                    y: c.y,
                                    z: c.z,
                                };
                                let corpse = death::Corpse::new(&p, at, c.orientation);
                                self.add_corpse(corpse);
                            }
                        }
                        // Died and logged out before releasing: up again.
                        None => p.row.state.dead = false,
                    }
                }
                self.insert_player(p);
                eviction::logged_in(self, guid);
                spells::apply_passives(self, guid);
                spell_areas::reconcile(self, guid);
                pets::owner_arrived(self, guid);
                npc::resume_flight(self, guid);
                social::logged_in(self, guid);
                self.visibility_dirty.insert(guid);
            }
            ArrivalKind::Transfer(mut p) => {
                eviction::transferred(&self.realm, &mut p, self.key);
                p.row.map = self.key.map;
                p.move_check = None;
                p.fall_from = None;
                p.awaiting_worldport = true;
                p.teleport = None;
                p.outbound
                    .send(&wow_world_messages::vanilla::SMSG_NEW_WORLD {
                        map: crate::game::map::movement::map_enum(self.key.map),
                        position: p.position(),
                        orientation: p.movement.orientation,
                    });
                crate::game::instances::send_saved(&self.realm, p.guid, &p.outbound);
                let guid = p.guid;
                self.insert_player(*p);
                corpses::arrived(self, guid);
                pets::owner_arrived(self, guid);
            }
        }
    }

    fn insert_player(&mut self, mut p: Player) {
        p.known.clear();
        p.observers.clear();
        p.cell = grid::cell_of(p.position().x, p.position().y);
        p.vis_anchor = p.position();
        self.grid.insert(p.guid, p.cell);
        self.players.insert(p.guid, p);
    }

    /// Take a player out of the map: every client that knows them forgets
    /// them, and they forget everything.
    pub fn take_player(&mut self, guid: Guid) -> Option<Player> {
        pets::owner_leaving(self, guid);
        battleground::departing(self, guid);
        duel::left(self, guid, true);
        trade::cancel(
            self,
            guid,
            wow_world_messages::vanilla::SMSG_TRADE_STATUS_TradeStatus::TargetLogout,
        );
        let mut p = self.players.remove(&guid)?;
        self.grid.remove(guid, p.cell);
        self.visibility_dirty.remove(&guid);
        self.batches.remove(&guid);
        self.forget.remove(&guid);
        for observer in p.observers.drain() {
            if let Some(o) = self.players.get_mut(&observer) {
                o.known.remove(&guid);
                self.forget.entry(observer).or_default().push(guid);
            }
        }
        for known in p.known.drain() {
            self.remove_observer(known, guid);
        }
        Some(p)
    }

    /// Store a player's character and take them off the online list.
    pub fn save_and_go_offline(&mut self, mut p: Player) {
        let row = p.to_row(self.now);
        self.realm.store.save(row);
        social::logged_out(self, p.guid);
        self.realm.registry.go_offline(p.guid);
    }

    pub fn save_player(&mut self, guid: Guid) {
        pets::sync_record(self, guid);
        let now = self.now;
        if let Some(p) = self.players.get_mut(&guid) {
            let row = p.to_row(now);
            p.save_at = now + self.realm.config.save_interval;
            self.realm.store.save(row);
        }
    }

    /// The map's numbers for the last while: players, creatures, update
    /// times and the events waiting.
    fn report(&mut self, queued: usize) {
        let s = &mut self.stats;
        if self.players.is_empty() && s.window_max < UPDATE_INTERVAL {
            *s = MapStats {
                updates: s.updates,
                events: s.events,
                last_update: s.last_update,
                max_update: s.max_update,
                panics: s.panics,
                ..Default::default()
            };
            return;
        }
        let avg = s.window_total.as_secs_f64() * 1000.0 / f64::from(s.window_updates.max(1));
        info!(
            "{}: {} players, {} creatures, {} objects; updates {:.2} ms average, {:.1} ms at most; {} events waiting",
            self.key,
            self.players.len(),
            self.creatures.len(),
            self.gameobjects.len(),
            avg,
            s.window_max.as_secs_f64() * 1000.0,
            queued,
        );
        s.window_updates = 0;
        s.window_total = Duration::ZERO;
        s.window_max = Duration::ZERO;
    }

    /// A copy empty long enough unloads.
    fn unload_if_empty(&mut self) {
        if self.key.instance == 0 {
            return;
        }
        if !self.players.is_empty() {
            self.empty_since = None;
            return;
        }
        let since = *self.empty_since.get_or_insert(self.now);
        if self.now.duration_since(since) >= self.realm.config.instance_unload {
            self.realm.maps.stop(self.key);
            crate::game::instances::unloaded(&self.realm, self.key);
            crate::game::battlegrounds::closed(&self.realm, self.key);
            self.stopping = true;
        }
    }

    /// Send a player to `key` at `position`, within this map or to another.
    pub fn teleport(
        &mut self,
        guid: Guid,
        key: MapKey,
        position: wow_world_messages::vanilla::Vector3d,
        orientation: f32,
    ) {
        let mut key = key;
        if key.instance == 0 && key.map != self.key.map {
            let gm = self.players.get(&guid).is_some_and(|p| p.gm);
            match crate::game::instances::key_for(&self.realm, guid, gm, key.map) {
                Ok(k) => key = k,
                Err(refused) => {
                    if let Some(p) = self.players.get(&guid) {
                        match refused {
                            crate::game::instances::Refused::Abort(reason) => p.outbound.send(
                                &wow_world_messages::vanilla::SMSG_TRANSFER_ABORTED {
                                    map: movement::map_enum(key.map),
                                    reason,
                                    argument: 0,
                                },
                            ),
                            crate::game::instances::Refused::Message(message) => p.outbound.send(
                                &wow_world_messages::vanilla::SMSG_AREA_TRIGGER_MESSAGE { message },
                            ),
                            crate::game::instances::Refused::RaidGroupOnly => p.outbound.send(
                                &wow_world_messages::vanilla::SMSG_RAID_GROUP_ONLY {
                                    homebind_timer: 0,
                                    error: wow_world_messages::vanilla::RaidGroupError::Required,
                                },
                            ),
                        };
                    }
                    return;
                }
            }
        }
        if key == self.key {
            movement::teleport_within(self, guid, position, orientation);
            return;
        }
        let Some(p) = self.players.get(&guid) else {
            return;
        };
        p.outbound
            .send(&wow_world_messages::vanilla::SMSG_TRANSFER_PENDING {
                map: movement::map_enum(key.map),
                has_transport: None,
            });
        let Some(mut p) = self.take_player(guid) else {
            return;
        };
        p.movement.position = position;
        p.movement.orientation = orientation;
        p.movement.flags = wow_world_messages::vanilla::MovementInfo_MovementFlags::empty();
        p.logout_at = None;
        let session = p.online.session.clone();
        let _ = session.send(Control::Transfer {
            player: Box::new(p),
            to: key,
        });
    }

    /// The update: timers, visibility, and the changed fields of
    /// everything, to every client that should hear of them.
    pub fn update(&mut self) {
        let started = Instant::now();
        self.now = started;
        self.stats.updates += 1;

        self.update_players();
        inventory::sync_gear_all(self);
        misc::update(self);
        combat::update_players(self);
        player_motion::update(self);
        intoxication::update(self);
        item_scripts::update(self);
        pvp::update(self);
        outdoor_pvp::update(self);
        arena::update(self);
        npc::update_flights(self);
        spells::update(self);
        auras::update(self);
        boss_mechanics::update(self);
        loot::update(self);
        quests::update_timers(self);
        items::update(self);
        enchant::update(self);
        scripts::update(self);
        event_ai::update_despawns(self);
        instance_script::update(self);
        map_events::update(self);
        scripts::update_objects(self);
        objects::update(self);
        social::push_member_stats(self);
        duel::update(self);
        battleground::update(self);
        eviction::update(self);
        self.unload_if_empty();
        pets::update(self);
        ai::update_creatures(self);
        combat::update_creatures(self);
        escorts::update(self);
        crate::game::elemental::update(self);
        ai::update_dead(self);
        visibility::refresh_dirty(self);
        quests::update_objects(self);
        self.flush_forgets();
        self.flush_values();
        self.flush_batches();
        for (g, frame) in std::mem::take(&mut self.late_frames) {
            if let Some(p) = self.players.get(&g) {
                p.outbound.send_frame(frame);
            }
        }
        self.create_cache.clear();

        let took = started.elapsed();
        self.stats.last_update = took;
        self.stats.window_updates += 1;
        self.stats.window_total += took;
        self.stats.window_max = self.stats.window_max.max(took);
        self.stats.max_update = self.stats.max_update.max(took);
        if took > UPDATE_INTERVAL {
            warn!(
                "{} update took {} ms ({} players, {} creatures)",
                self.key,
                took.as_millis(),
                self.players.len(),
                self.creatures.len()
            );
        }
    }

    fn update_players(&mut self) {
        let now = self.now;
        let mut logouts = Vec::new();
        let mut saves = Vec::new();
        for p in self.players.values_mut() {
            if p.logout_at.is_some_and(|t| t <= now) {
                logouts.push(p.guid);
                continue;
            }
            if p.teleport
                .is_some_and(|t| now.duration_since(t.since) > TELEPORT_TIMEOUT)
            {
                p.teleport = None;
            }
            if p.save_at <= now {
                saves.push(p.guid);
            }
        }
        for g in logouts {
            login::complete_logout(self, g);
        }
        for g in saves {
            self.save_player(g);
        }
        stats::regen(self);
    }

    fn flush_forgets(&mut self) {
        for (observer, guids) in self.forget.drain() {
            if guids.is_empty() || !self.players.contains_key(&observer) {
                continue;
            }
            let block = update::out_of_range_block(&guids);
            self.batches.entry(observer).or_default().push(&block);
        }
    }

    fn flush_values(&mut self) {
        let batches = &mut self.batches;
        for p in self.players.values_mut() {
            if p.awaiting_worldport {
                continue;
            }
            for item in &mut p.items {
                if let Some(block) =
                    update::values_block(item.guid, &item.fields, Visibility::Owner)
                {
                    batches.entry(p.guid).or_default().push(&block);
                }
                item.fields.clear_dirty();
            }
            if !p.fields.is_dirty() {
                continue;
            }
            if let Some(block) = update::values_block(p.guid, &p.fields, Visibility::Owner) {
                batches.entry(p.guid).or_default().push(&block);
            }
            if !p.observers.is_empty()
                && let Some(block) = update::values_block(p.guid, &p.fields, Visibility::Public)
            {
                for o in &p.observers {
                    batches.entry(*o).or_default().push(&block);
                }
            }
            p.fields.clear_dirty();
        }
        for (_, o) in self.gameobjects.iter_mut() {
            if !o.fields.is_dirty() {
                continue;
            }
            if !o.observers.is_empty()
                && let Some(block) = update::values_block(o.guid, &o.fields, Visibility::Public)
            {
                for p in &o.observers {
                    batches.entry(*p).or_default().push(&block);
                }
            }
            o.fields.clear_dirty();
        }
        for (_, c) in self.creatures.iter_mut() {
            if !c.fields.is_dirty() {
                continue;
            }
            if !c.observers.is_empty()
                && let Some(block) = update::values_block(c.guid, &c.fields, Visibility::Public)
            {
                for o in &c.observers {
                    batches.entry(*o).or_default().push(&block);
                }
            }
            c.fields.clear_dirty();
        }
    }

    fn flush_batches(&mut self) {
        for (guid, batch) in self.batches.drain() {
            let Some(p) = self.players.get(&guid) else {
                continue;
            };
            for frame in batch.finish() {
                p.outbound.send_frame(frame);
            }
        }
    }

    /// The create block for `object` as a stranger sees it, built once per
    /// update.
    pub fn public_create(&mut self, object: Guid) -> Option<Arc<[u8]>> {
        if let Some(b) = self.create_cache.get(&object) {
            return Some(b.clone());
        }
        let now = self.now;
        let block: Arc<[u8]> = if guid::is_player(object) {
            let p = self.players.get(&object)?;
            Arc::from(update::create_block(&p.create(false), Visibility::Public))
        } else if guid::is_unit(object) {
            let c = self.creature(object)?;
            Arc::from(update::create_block(&c.create(now), Visibility::Public))
        } else if guid::is_game_object(object) {
            let g = self.gameobject(object)?;
            Arc::from(update::create_block(&g.create(), Visibility::Public))
        } else if guid::is_corpse(object) {
            let c = self.corpses.get(&object)?;
            Arc::from(update::create_block(&c.create(), Visibility::Public))
        } else if spells::dynamic::is_dynamic_object(object) {
            let d = self.spells.dynamic.get(&object)?;
            Arc::from(update::create_block(&d.create(), Visibility::Public))
        } else {
            return None;
        };
        self.create_cache.insert(object, block.clone());
        Some(block)
    }

    pub fn creature(&self, guid: Guid) -> Option<&Creature> {
        self.creature_by_guid
            .get(&guid)
            .and_then(|k| self.creatures.get(*k))
    }

    pub fn creature_mut(&mut self, guid: Guid) -> Option<&mut Creature> {
        let k = *self.creature_by_guid.get(&guid)?;
        self.creatures.get_mut(k)
    }

    pub fn gameobject(&self, guid: Guid) -> Option<&GameObject> {
        self.gameobject_by_guid
            .get(&guid)
            .and_then(|k| self.gameobjects.get(*k))
    }

    pub fn gameobject_mut(&mut self, guid: Guid) -> Option<&mut GameObject> {
        let k = *self.gameobject_by_guid.get(&guid)?;
        self.gameobjects.get_mut(k)
    }

    pub fn add_creature(&mut self, c: Creature) -> Guid {
        let guid = c.guid;
        let cell = c.cell;
        if let Some(s) = c.spawn_guid {
            self.by_spawn.insert(s, guid);
        }
        if event_ai::has_events_entry(&self.realm.data, c.entry) || self.instance.script.is_some() {
            self.ai_spawned.push(guid);
        }
        let key = self.creatures.insert(c);
        self.creature_by_guid.insert(guid, key);
        self.grid.insert(guid, cell);
        self.visibility_dirty.insert(guid);
        spawn_auras::apply(self, guid);
        guid
    }

    pub fn add_gameobject(&mut self, g: GameObject) -> Guid {
        let guid = g.guid;
        let cell = g.cell;
        let key = self.gameobjects.insert(g);
        self.gameobject_by_guid.insert(guid, key);
        self.grid.insert(guid, cell);
        self.visibility_dirty.insert(guid);
        guid
    }

    pub fn remove_gameobject(&mut self, guid: Guid) -> Option<GameObject> {
        let key = self.gameobject_by_guid.remove(&guid)?;
        let mut g = self.gameobjects.try_remove(key)?;
        self.grid.remove(guid, g.cell);
        for o in g.observers.drain() {
            if let Some(p) = self.players.get_mut(&o) {
                p.known.remove(&guid);
                self.forget.entry(o).or_default().push(guid);
            }
        }
        Some(g)
    }

    pub fn remove_creature(&mut self, guid: Guid) -> Option<Creature> {
        let key = self.creature_by_guid.remove(&guid)?;
        let mut c = self.creatures.try_remove(key)?;
        if let Some(s) = c.spawn_guid
            && self.by_spawn.get(&s) == Some(&guid)
        {
            self.by_spawn.remove(&s);
        }
        self.script_groups.remove(&guid);
        self.grid.remove(guid, c.cell);
        for o in c.observers.drain() {
            if let Some(p) = self.players.get_mut(&o) {
                p.known.remove(&guid);
                self.forget.entry(o).or_default().push(guid);
            }
        }
        Some(c)
    }

    /// The position of any object in the map.
    pub fn position_of(&self, object: Guid) -> Option<wow_world_messages::vanilla::Vector3d> {
        if guid::is_player(object) {
            self.players.get(&object).map(|p| p.position())
        } else if guid::is_unit(object) {
            self.creature(object).map(|c| c.position)
        } else if guid::is_game_object(object) {
            self.gameobject(object).map(|g| g.position)
        } else if guid::is_corpse(object) {
            self.corpses.get(&object).map(|c| c.position)
        } else if spells::dynamic::is_dynamic_object(object) {
            self.spells.dynamic.get(&object).map(|d| d.position)
        } else {
            None
        }
    }

    /// The players whose clients know `object`.
    pub fn observers(&self, object: Guid) -> Option<&ahash::AHashSet<Guid>> {
        if guid::is_player(object) {
            self.players.get(&object).map(|p| &p.observers)
        } else if guid::is_unit(object) {
            self.creature(object).map(|c| &c.observers)
        } else if guid::is_game_object(object) {
            self.gameobject(object).map(|g| &g.observers)
        } else if guid::is_corpse(object) {
            self.corpses.get(&object).map(|c| &c.observers)
        } else if spells::dynamic::is_dynamic_object(object) {
            self.spells.dynamic.get(&object).map(|d| &d.observers)
        } else {
            None
        }
    }

    pub fn observers_mut(&mut self, object: Guid) -> Option<&mut ahash::AHashSet<Guid>> {
        if guid::is_player(object) {
            self.players.get_mut(&object).map(|p| &mut p.observers)
        } else if guid::is_unit(object) {
            self.creature_mut(object).map(|c| &mut c.observers)
        } else if guid::is_game_object(object) {
            self.gameobject_mut(object).map(|g| &mut g.observers)
        } else if guid::is_corpse(object) {
            self.corpses.get_mut(&object).map(|c| &mut c.observers)
        } else if spells::dynamic::is_dynamic_object(object) {
            self.spells.dynamic.get_mut(&object).map(|d| &mut d.observers)
        } else {
            None
        }
    }

    /// `observer` no longer knows `object`.
    pub fn remove_observer(&mut self, object: Guid, observer: Guid) {
        if let Some(o) = self.observers_mut(object) {
            o.remove(&observer);
        }
    }

    /// Send `frame` to every client that knows `object`, and to the object
    /// itself if it is a player and `to_self` is set.
    pub fn send_to_observers(&self, object: Guid, frame: &Arc<[u8]>, to_self: bool) {
        if to_self && let Some(p) = self.players.get(&object) {
            p.outbound.send_frame(frame.clone());
        }
        let Some(observers) = self.observers(object) else {
            return;
        };
        for o in observers {
            if let Some(p) = self.players.get(o) {
                p.outbound.send_frame(frame.clone());
            }
        }
    }

    /// Players within `radius` yards of `object`, the object included if it
    /// is one.
    pub fn players_near(&self, x: f32, y: f32, radius: f32) -> Vec<Guid> {
        let r2 = radius * radius;
        self.grid
            .around(x, y, radius)
            .filter(|g| guid::is_player(*g))
            .filter(|g| {
                self.players.get(g).is_some_and(|p| {
                    let d = p.position();
                    (d.x - x).powi(2) + (d.y - y).powi(2) <= r2
                })
            })
            .collect()
    }

    pub fn player(&self, guid: Guid) -> Option<&Player> {
        self.players.get(&guid)
    }

    pub fn player_mut(&mut self, guid: Guid) -> Option<&mut Player> {
        self.players.get_mut(&guid)
    }

    /// A system message to one player.
    pub fn system_message(&self, guid: Guid, text: &str) {
        if let Some(p) = self.players.get(&guid) {
            chat::send_system(&p.outbound, text);
        }
    }
}
