//! Maps: each map, or instance of one, is an actor that owns everything in
//! it, handles its players' packets as they arrive, and runs a 20 Hz update
//! for what time drives.

pub mod account_data;
pub mod ai;
pub mod arena;
pub mod assistance;
pub mod auction;
pub mod auras;
pub mod battleground;
pub mod boss_mechanics;
pub mod charge;
pub mod chat;
pub mod cinematic;
pub mod combat;
pub mod conditions;
pub mod corpses;
pub mod creature;
pub mod creature_combat;
pub mod creature_groups;
pub mod creature_motion;

pub mod death;
pub mod derive;
pub mod duel;
pub mod enchant;
pub mod environment;
pub mod escorts;
pub mod event_ai;
pub mod event_quests;
pub mod eviction;
pub mod exploration;
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
pub mod item_rules;
pub mod item_scripts;
pub mod items;
pub mod items_misc;
pub mod local_defense;
pub mod login;
pub mod loot;
pub mod map_events;
pub mod meeting_stones;
pub mod misc;
pub mod mounts;
pub mod movement;
pub mod nav;
pub mod npc;
pub mod objects;
pub mod outdoor_pvp;
pub mod pets;
pub mod player;
pub mod player_motion;
pub mod pools;
pub mod proc_scripts;
pub mod procs;
pub mod professions;
pub mod pvp;
pub mod query;
pub mod quests;
pub mod questshare;
pub mod ranged;
pub mod reputation;
pub mod rest;
pub mod scripts;
pub mod skills;
pub mod social;
pub mod spawn_auras;
pub mod spell_areas;
pub mod spells;
pub mod stats;
pub mod supervise;
pub mod tap;
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
    /// What the actor tells the watchdog.
    pulse: Arc<crate::game::supervisor::Pulse>,
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

    /// Whether the actor has gone: what is sent is lost.
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    pub fn pulse(&self) -> &crate::game::supervisor::Pulse {
        &self.pulse
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
    /// The server is shutting down: every map has saved its players.
    closed: std::sync::atomic::AtomicBool,
}

impl Maps {
    /// The map's handle, starting its actor if it isn't running, or has
    /// gone (a supervisor's restart of a child that died).
    pub fn get(&self, realm: &Arc<Realm>, key: MapKey) -> MapHandle {
        if let Some(h) = self.handles.read().get(&key)
            && !h.is_closed()
        {
            return h.clone();
        }
        let mut handles = self.handles.write();
        if let Some(h) = handles.get(&key) {
            if !h.is_closed() {
                return h.clone();
            }
            tracing::error!("{key}'s actor has gone; starting it again");
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = MapHandle {
            key,
            tx,
            pulse: Default::default(),
        };
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

    /// The server is shutting down; the maps save everyone and stop.
    pub fn close(&self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// How often each map logs its numbers.
pub const REPORT_EVERY: Duration = Duration::from_secs(60);

/// The map update's period.
pub const UPDATE_INTERVAL: Duration = Duration::from_millis(50);
/// How long a client has to acknowledge a teleport before its movement is
/// heeded again.
const TELEPORT_TIMEOUT: Duration = Duration::from_secs(5);

/// What Tracy shows of a map (docs/PROFILING.md): its updates, a frame
/// each, and four plots, once a second.
struct MapProfile {
    frame: crate::tracy::FrameName,
    players: crate::tracy::Plot,
    awake: crate::tracy::Plot,
    mailbox: crate::tracy::Plot,
    outbound: crate::tracy::Plot,
    plot_at: Instant,
}

impl MapProfile {
    /// `None` while Tracy is off. A map by its name, a copy by its name and
    /// instance id.
    fn new(key: MapKey, realm: &Realm) -> Option<Self> {
        use crate::tracy::{Format, FrameName, Plot};
        if !crate::tracy::on() {
            return None;
        }
        let name = realm
            .data
            .maps
            .get(&key.map)
            .map_or_else(|| format!("map {}", key.map), |m| m.name.clone());
        let name = if key.instance == 0 {
            name
        } else {
            format!("{name} #{}", key.instance)
        };
        Some(Self {
            frame: FrameName::new(&name),
            players: Plot::new(&format!("{name}: players"), Format::Number),
            awake: Plot::new(&format!("{name}: creatures awake"), Format::Number),
            mailbox: Plot::new(&format!("{name}: mailbox"), Format::Number),
            outbound: Plot::new(&format!("{name}: outbound queued"), Format::Bytes),
            plot_at: Instant::now(),
        })
    }
}

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
    /// Events, update steps and creatures' turns that panicked and were
    /// let go.
    pub panics: u64,
    /// Creatures that kept panicking, despawned for good.
    pub quarantined: u32,
    /// Times the map was built again after its update kept panicking.
    pub restarts: u32,
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
    /// The game objects (their keys) that may have changed since the last
    /// update's flush: every change to one goes through
    /// `Map::gameobject_mut`, or comes with `Map::add_gameobject`.
    touched_objects: Vec<usize>,
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
    /// Formations whose leader died, by the leader's spawn: the member
    /// leading their patrol meanwhile.
    pub acting_leaders: AHashMap<u32, Guid>,
    /// Linked slaves' spawns held back by their master's life, to come in
    /// once it allows (vmangos's `CanSpawn`, Thistle Tea's `:held`).
    pub held_slaves: ahash::AHashSet<u32>,
    /// The spawn pools here, by their root: what each has picked.
    pub pools: AHashMap<u32, pools::PoolState>,
    /// Scripted map events under way.
    pub map_events: map_events::Events,
    pub spells: spells::MapSpells,
    /// Which of the rarer states a creature here may be in (a charge, a
    /// flight, a call for help, a cast, a pet), so the update can skip its
    /// looks for them.
    pub rare: creature_combat::RareStates,
    /// Creatures that may be awake: seen by a player, or in a fight
    /// (attacking, attacked, evading, or down in a feign death). Every
    /// creature that is, and some that were: a player coming to know one
    /// adds it, as does [`Map::note_fighting`], and [`Map::awake_where`]
    /// lets go of those no longer awake. The update's looks for what only
    /// an awake creature could be go through it, not every creature.
    pub awake: ahash::AHashSet<Guid>,
    /// Creatures that may have auras: every creature that has, and some
    /// that had. An aura is only ever added through `auras::auras_mut`,
    /// which adds its creature, as `Map::add_creature` adds one that
    /// arrives with auras; `Map::with_auras_where` lets go of those
    /// without.
    pub with_auras: ahash::AHashSet<Guid>,
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
    /// Scripted area triggers resting from their cooldown, until when.
    pub trigger_cooldowns: AHashMap<u32, Instant>,
    /// The leash clocks creatures in one fight share.
    pub leashes: creature_combat::Leashes,
    /// Who has panicked lately, and when the map has had enough.
    pub health: supervise::Health,
    /// What Tracy shows of the map, while Tracy is on.
    profile: Option<MapProfile>,
    /// Panics a test arms.
    #[cfg(test)]
    pub faults: crate::game::supervisor::Faults,
}

/// What `out` makes of each creature of `set` that `pred` takes, in key
/// order (as a look through every creature goes); members gone, or no
/// longer `kept`, are let go.
fn members_where<T>(
    set: &mut ahash::AHashSet<Guid>,
    creatures: &Slab<Creature>,
    by_guid: &AHashMap<Guid, usize>,
    kept: impl Fn(&Creature) -> bool,
    pred: impl Fn(&Creature) -> bool,
    out: impl Fn(usize, &Creature) -> T,
) -> Vec<T> {
    let mut keys = Vec::with_capacity(set.len());
    set.retain(
        |g| match by_guid.get(g).and_then(|&k| Some((k, creatures.get(k)?))) {
            Some((k, c)) if kept(c) => {
                keys.push(k);
                true
            }
            _ => false,
        },
    );
    keys.sort_unstable();
    keys.into_iter()
        .filter_map(|k| Some((k, creatures.get(k)?)))
        .filter(|(_, c)| pred(c))
        .map(|(k, c)| out(k, c))
        .collect()
}

/// What `out` makes of each creature `pred` takes, looking at every one:
/// what the shortcuts must find, for debug builds' checks.
fn every_creature_where<T>(
    creatures: &Slab<Creature>,
    pred: impl Fn(&Creature) -> bool,
    out: impl Fn(usize, &Creature) -> T,
) -> Vec<T> {
    creatures
        .iter()
        .filter(|(_, c)| pred(c))
        .map(|(k, c)| out(k, c))
        .collect()
}

/// Whether `c` is awake (see `Map::awake`): seen, or in a fight.
pub fn awake(c: &Creature) -> bool {
    !c.observers.is_empty()
        || c.combat.attacking
        || !c.combat.attackers.is_empty()
        || c.evading
        || c.scripted
            .as_deref()
            .is_some_and(|s| s.feign.is_some_and(|f| f.down_at.is_some()))
}

/// Run `f` on the map; a panic in it is logged and the map goes on, so a
/// bad packet or a bug costs one event or one update, not the map.
fn guarded<T>(map: &mut Map, what: &str, f: impl FnOnce(&mut Map) -> T) -> Option<T> {
    match crate::game::supervisor::catch(|| f(map)) {
        Ok(v) => Some(v),
        Err(e) => {
            map.stats.panics += 1;
            if crate::game::supervisor::worth_logging(map.stats.panics) {
                tracing::error!(
                    "{}: {what} panicked: {e}; going on ({} panics)",
                    map.key,
                    map.stats.panics
                );
            }
            None
        }
    }
}

/// Handle one event, under guard: true to stop the actor. A player's
/// packet that panics counts against them.
fn handle_guarded(map: &mut Map, event: MapEvent) -> bool {
    let from = match &event {
        MapEvent::Packet(g, _) => Some(*g),
        _ => None,
    };
    match guarded(map, "an event", |m| m.handle_event(event)) {
        Some(stop) => stop,
        None => {
            if let Some(g) = from {
                supervise::packet_panicked(map, g);
            }
            false
        }
    }
}

/// Run a map's actor until it is told to stop.
async fn run(mut map: Map, mut rx: mpsc::UnboundedReceiver<MapEvent>) {
    guarded(&mut map, "populating", |m| {
        let _z = crate::zone!("populate");
        world::populate(m)
    });
    let mut report_at = Instant::now() + REPORT_EVERY;
    let mut ticker = tokio::time::interval(UPDATE_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        map.handle.pulse.beat(rx.len());
        tokio::select! {
            event = rx.recv() => {
                let Some(event) = event else { break };
                map.now = Instant::now();
                if handle_guarded(&mut map, event) {
                    break;
                }
                // Drain what else has arrived, a bounded amount, so the
                // update isn't starved.
                for _ in 0..512 {
                    match rx.try_recv() {
                        Ok(event) => {
                            if handle_guarded(&mut map, event) {
                                hand_on(&map, &mut rx);
                                return;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
            _ = ticker.tick() => {
                guarded(&mut map, "the update", Map::update);
                if map.stopping {
                    break;
                }
                // An update that keeps panicking: a bad state, that a fresh
                // map from the data won't have.
                if supervise::escalated(&map) {
                    map = supervise::restart(map);
                    continue;
                }
                if map.now >= report_at {
                    report_at = map.now + REPORT_EVERY;
                    map.report(rx.len());
                }
                map.plot(rx.len());
            }
        }
    }
    hand_on(&map, &mut rx);
}

/// What was sent to a map that has stopped (a copy unloaded or reset as a
/// player was on their way in) isn't lost with it: a player arriving goes
/// on into the map started afresh, or, as the server shuts down, is saved
/// and let go. Before, they were dropped, unsaved and still online.
fn hand_on(map: &Map, rx: &mut mpsc::UnboundedReceiver<MapEvent>) {
    let rest: Vec<MapEvent> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter(|e| !matches!(e, MapEvent::Stop | MapEvent::Shutdown(_)))
        .collect();
    if !rest.iter().any(|e| matches!(e, MapEvent::Arrive(_))) {
        return;
    }
    let realm = &map.realm;
    // Shutting down, or a copy its raid's reset took (which would stop
    // again at once): saved where they were going, and let go.
    if realm.maps.is_closed() || crate::game::instances::expired(realm, map.key) {
        for e in rest {
            let MapEvent::Arrive(a) = e else { continue };
            let Arrival { kind, online } = *a;
            if let ArrivalKind::Transfer(mut p) = kind {
                p.row.map = map.key.map;
                realm.store.save(p.to_row(Instant::now()));
            }
            realm.registry.go_offline(online.guid);
            let _ = online.session.send(crate::game::session::Control::Kick);
        }
        return;
    }
    warn!(
        "{} stopped with a player on the way in; starting it again",
        map.key
    );
    let next = realm.maps.get(realm, map.key);
    for e in rest {
        next.send(e);
    }
}

impl Map {
    pub fn new(key: MapKey, realm: Arc<Realm>, handle: MapHandle) -> Self {
        let now = Instant::now();
        let instance = instance_script::State::for_map(&realm.data, key.map, key.instance);
        let profile = MapProfile::new(key, &realm);
        Self {
            key,
            realm,
            handle,
            players: AHashMap::new(),
            creatures: Slab::new(),
            creature_by_guid: AHashMap::new(),
            gameobjects: Slab::new(),
            gameobject_by_guid: AHashMap::new(),
            touched_objects: Vec::new(),
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
            acting_leaders: AHashMap::new(),
            held_slaves: Default::default(),
            pools: AHashMap::new(),
            map_events: Default::default(),
            spells: Default::default(),
            rare: Default::default(),
            awake: ahash::AHashSet::new(),
            with_auras: ahash::AHashSet::new(),
            escorts: Vec::new(),
            rifts: AHashMap::new(),
            rifts_checked_at: None,
            arena: Default::default(),
            objects: Default::default(),
            late_frames: Vec::new(),
            trigger_cooldowns: AHashMap::new(),
            leashes: Default::default(),
            health: Default::default(),
            profile,
            #[cfg(test)]
            faults: Default::default(),
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
            MapEvent::Arrive(a) => {
                let _z = crate::zone!("arrival");
                self.arrive(*a)
            }
            MapEvent::Packet(guid, msg) => {
                // wow_messages names a movement packet from the client
                // `MSG_MOVE_HEARTBEAT_Client`; the opcode is `MSG_MOVE_HEARTBEAT`.
                let name = msg.message_name();
                let _z = crate::tracy::Zone::named(
                    "packet",
                    name.strip_suffix("_Client").unwrap_or(name),
                );
                #[cfg(test)]
                if self.faults.any() {
                    self.faults.check(&format!("packets of {guid}"));
                }
                self.handle_packet(guid, msg)
            }
            MapEvent::Disconnect(guid) => {
                let _z = crate::zone!("disconnect");
                self.handle_disconnect(guid)
            }
            MapEvent::Call(f) => {
                let _z = crate::zone!("call");
                f(self)
            }
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

    /// A player's client has gone: they leave, saved.
    pub fn handle_disconnect(&mut self, guid: Guid) {
        if let Some(p) = self.take_player(guid) {
            debug!("{} disconnected from {}", p.name, self.key);
            self.save_and_go_offline(p);
        }
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
                let mailed = inventory::unusable_off_on_login(&mut p, &self.realm.data.clone());
                if !mailed.is_empty() {
                    p.save_at = self.now;
                }
                for item in mailed {
                    inventory::mail_back(&self.realm, guid, item);
                }
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
                                let corpse =
                                    death::Corpse::new(&self.realm.data, &p, at, c.orientation);
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
        // The client's first move is checked from where it came in.
        p.move_check = Some((p.position(), self.now));
        self.grid.insert(p.guid, p.cell);
        self.players.insert(p.guid, p);
    }

    /// Take a player out of the map: every client that knows them forgets
    /// them, and they forget everything.
    pub fn take_player(&mut self, guid: Guid) -> Option<Player> {
        // Their cast or channel ends, and the creatures they fought forget
        // them (mangos's `RemoveFromWorld`: the cast interrupted, the
        // hostile references deleted), as a death has them forgotten.
        if self.caster(guid).is_some_and(|c| c.casting.is_some()) {
            spells::interrupt(self, guid);
        }
        ai::forget(self, guid);
        pets::owner_leaving(self, guid);
        spells::single_target::caster_gone(self, guid);
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

    /// Save the players and records an item or money just moved between,
    /// in one transaction (`Store::save_together`), so that a crash after
    /// it neither loses nor copies what moved. Each player's periodic save
    /// waits its interval again.
    pub fn save_together(&mut self, players: &[Guid], records: crate::game::store::Records) {
        let now = self.now;
        let mut rows = Vec::with_capacity(players.len());
        for &guid in players {
            pets::sync_record(self, guid);
            if let Some(p) = self.players.get_mut(&guid) {
                rows.push(p.to_row(now));
                p.save_at = now + self.realm.config.save_interval;
            }
        }
        self.realm.store.save_together(rows, records);
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

    /// Tracy's plots of the map, once a second while Tracy is on.
    fn plot(&mut self, queued: usize) {
        let Some(p) = &mut self.profile else {
            return;
        };
        if self.now < p.plot_at {
            return;
        }
        p.plot_at = self.now + Duration::from_secs(1);
        let outbound: usize = self
            .players
            .values()
            .map(|p| p.outbound.queued_bytes())
            .sum();
        p.players.set(self.players.len() as f64);
        p.awake.set(self.awake.len() as f64);
        p.mailbox.set(queued as f64);
        p.outbound.set(outbound as f64);
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
                quarantined: s.quarantined,
                restarts: s.restarts,
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
        // Just emptied: the save is touched, so the realm's sweep counts
        // its empty time from now, as this copy's own clock does.
        if self.empty_since.is_none() {
            crate::game::instances::touch(&self.realm, self.key);
        }
        let since = *self.empty_since.get_or_insert(self.now);
        // One its raid's reset took goes at once.
        if crate::game::instances::expired(&self.realm, self.key)
            || self.now.duration_since(since) >= self.realm.config.instance_unload
        {
            self.realm.maps.stop(self.key);
            crate::game::instances::unloaded(&self.realm, self.key);
            crate::game::battlegrounds::closed(&self.realm, self.key);
            self.stopping = true;
        }
    }

    /// Send a player to `key` at `position`, within this map or to another.
    /// A map named without its copy is this copy when it is this map (a
    /// spell's place within a dungeon, BWL's orb), or the player's copy of
    /// another.
    pub fn teleport(
        &mut self,
        guid: Guid,
        key: MapKey,
        position: wow_world_messages::vanilla::Vector3d,
        orientation: f32,
    ) {
        let mut key = key;
        if key.instance == 0 && key.map == self.key.map {
            key = self.key;
        }
        if key.instance == 0 && key.map != self.key.map {
            let gm = self.players.get(&guid).is_some_and(|p| p.gm);
            match crate::game::instances::key_for(&self.realm, guid, gm, key.map) {
                Ok(k) => key = k,
                Err(refused) => {
                    if let Some(p) = self.players.get(&guid) {
                        match refused {
                            // 1.12's is the reason's byte alone, as
                            // mangos-zero's SendTransferAborted and Thistle
                            // Tea write it.
                            crate::game::instances::Refused::Abort(reason) => p.outbound.send(
                                &wow_world_messages::vanilla::SMSG_TRANSFER_ABORTED { reason },
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
        // What a teleport ends, and leaving the map: a player's cast under
        // way too (Thistle Tea's `disengage_for_world_transition`), and a
        // flight (its `cancel_authoritative_movement`).
        npc::cancel_flight(self, guid);
        crate::game::map::spells::interrupts::teleported(self, guid);
        if self.players.contains_key(&guid)
            && self.caster(guid).is_some_and(|c| c.casting.is_some())
        {
            crate::game::map::spells::interrupt(self, guid);
        }
        if key == self.key {
            movement::teleport_within(self, guid, position, orientation);
            return;
        }
        crate::game::map::spells::interrupts::changed_map(self, guid);
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
        crate::game::session::transfer(&self.realm, &session, p, key);
    }

    /// The update: timers, visibility, and the changed fields of
    /// everything, to every client that should hear of them.
    pub fn update(&mut self) {
        let _frame = crate::tracy::Frame::begin(self.profile.as_ref().map(|p| p.frame));
        let z = crate::zone!("update");
        z.value(u64::from(self.key.map));
        let started = Instant::now();
        self.now = started;
        self.stats.updates += 1;

        // Each step under its own guard: one that panics costs itself,
        // this once, and the rest (the flushes most of all) go on.
        self.step("players", Map::update_players);
        self.step("gear", inventory::sync_gear_all);
        self.step("misc", misc::update);
        self.step("players' combat", combat::update_players);
        self.step("players' motion", player_motion::update);
        self.step("intoxication", intoxication::update);
        self.step("item scripts", item_scripts::update);
        self.step("pvp", pvp::update);
        self.step("outdoor pvp", outdoor_pvp::update);
        self.step("arena", arena::update);
        self.step("flights", npc::update_flights);
        self.step("spells", spells::update);
        self.step("auras", auras::update);
        self.step("boss mechanics", boss_mechanics::update);
        self.step("loot", loot::update);
        self.step("quest timers", quests::update_timers);
        self.step("items", items::update);
        self.step("enchantments", enchant::update);
        self.step("scripts", scripts::update);
        self.step("despawns", event_ai::update_despawns);
        self.step("instance script", instance_script::update);
        self.step("map events", map_events::update);
        self.step("object scripts", scripts::update_objects);
        self.step("objects", objects::update);
        self.step("member stats", social::push_member_stats);
        self.step("duels", duel::update);
        self.step("battleground", battleground::update);
        self.step("eviction", eviction::update);
        self.step("unloading", Map::unload_if_empty);
        self.step("pets", pets::update);
        self.step("charges", charge::update);
        self.step("creatures", ai::update_creatures);
        self.step("creatures' combat", combat::update_creatures);
        self.step("escorts", escorts::update);
        self.step("elemental invasion", crate::game::elemental::update);
        self.step("the dead", ai::update_dead);
        self.step("visibility", visibility::refresh_dirty);
        self.step("quest objects", quests::update_objects);
        self.step("forgets", Map::flush_forgets);
        self.step("values", Map::flush_values);
        self.step("batches", Map::flush_batches);
        self.step("late frames", Map::flush_late_frames);
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

    /// What each client is to forget, from what it was told this update:
    /// an object it had at the update's start is forgotten (first, before
    /// any create), and what it won't have at the end is dropped from its
    /// batch. One it learned of and lost within the update is neither.
    fn flush_forgets(&mut self) {
        for (observer, guids) in self.forget.drain() {
            let Some(p) = self.players.get(&observer) else {
                continue;
            };
            let batch = self.batches.entry(observer).or_default();
            let mut seen: Vec<Guid> = Vec::with_capacity(guids.len());
            for &g in &guids {
                if seen.contains(&g) {
                    continue;
                }
                seen.push(g);
                // Each forget took it from `known`, each create put it there.
                let forgets = guids.iter().filter(|x| **x == g).count() as i64;
                let creates = batch.creates_of(g) as i64;
                let now = i64::from(p.known.contains(&g));
                if now + forgets - creates > 0 {
                    batch.forget(g);
                }
                batch.drop_for(g, now == 1);
            }
        }
    }

    fn flush_values(&mut self) {
        let batches = &mut self.batches;
        let z = crate::zone!("values: players");
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
        drop(z);
        let z = crate::zone!("values: game objects");
        // Only a game object changed since the last flush can have dirty
        // fields; in key order, as a look through every one goes.
        let mut touched = std::mem::take(&mut self.touched_objects);
        touched.sort_unstable();
        touched.dedup();
        for k in touched {
            let Some(o) = self.gameobjects.get_mut(k) else {
                continue;
            };
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
        debug_assert!(
            self.gameobjects.iter().all(|(_, o)| !o.fields.is_dirty()),
            "{}: a game object changed, but not through Map::gameobject_mut",
            self.key
        );
        drop(z);
        // Every creature, for its dirty flag.
        let z = crate::zone!("values: creatures");
        // Creatures whose tap their observers see anew: taken or gone, or
        // their flags sent again to those outside it.
        let mut taps = Vec::new();
        for (_, c) in self.creatures.iter_mut() {
            let tap_changed = c.tapped_by != c.tap_told;
            if !c.fields.is_dirty() && !tap_changed {
                continue;
            }
            let flags_sent = c
                .fields
                .dirty()
                .get(crate::game::fields::UNIT_DYNAMIC_FLAGS);
            if !c.observers.is_empty()
                && let Some(block) = update::values_block(c.guid, &c.fields, Visibility::Public)
            {
                for o in &c.observers {
                    batches.entry(*o).or_default().push(&block);
                }
            }
            if !c.observers.is_empty() && (tap_changed || (flags_sent && c.tapped_by.is_some())) {
                taps.push((c.guid, tap_changed));
            }
            c.tap_told = c.tapped_by;
            c.fields.clear_dirty();
        }
        drop(z);
        for (g, all) in taps {
            tap::tell(self, g, all);
        }
    }

    fn flush_late_frames(&mut self) {
        for (g, frame) in std::mem::take(&mut self.late_frames) {
            if let Some(p) = self.players.get(&g) {
                p.outbound.send_frame(frame);
            }
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

    /// `g`, if a creature, may be in a fight from now (see `awake`).
    pub fn note_fighting(&mut self, g: Guid) {
        if self.creature_by_guid.contains_key(&g) {
            self.awake.insert(g);
        }
    }

    /// What `out` makes of each awake creature `pred` takes, in the order
    /// of a look through every creature, which is what debug builds check
    /// it finds (so `pred` takes only creatures that are awake). Those no
    /// longer awake are let go.
    pub fn awake_where<T: PartialEq + std::fmt::Debug>(
        &mut self,
        pred: impl Fn(&Creature) -> bool,
        out: impl Fn(usize, &Creature) -> T,
    ) -> Vec<T> {
        let found = members_where(
            &mut self.awake,
            &self.creatures,
            &self.creature_by_guid,
            awake,
            &pred,
            &out,
        );
        debug_assert_eq!(
            found,
            every_creature_where(&self.creatures, &pred, &out),
            "{}: a creature is awake but not in Map::awake",
            self.key
        );
        found
    }

    /// [`Map::awake_where`] for the creatures with auras (`with_auras`):
    /// `pred` takes only creatures with auras.
    pub fn with_auras_where<T: PartialEq + std::fmt::Debug>(
        &mut self,
        pred: impl Fn(&Creature) -> bool,
        out: impl Fn(usize, &Creature) -> T,
    ) -> Vec<T> {
        let found = members_where(
            &mut self.with_auras,
            &self.creatures,
            &self.creature_by_guid,
            |c| !c.auras.is_empty(),
            &pred,
            &out,
        );
        debug_assert_eq!(
            found,
            every_creature_where(&self.creatures, &pred, &out),
            "{}: a creature has auras but isn't in Map::with_auras",
            self.key
        );
        found
    }

    pub fn gameobject(&self, guid: Guid) -> Option<&GameObject> {
        self.gameobject_by_guid
            .get(&guid)
            .and_then(|k| self.gameobjects.get(*k))
    }

    pub fn gameobject_mut(&mut self, guid: Guid) -> Option<&mut GameObject> {
        let k = *self.gameobject_by_guid.get(&guid)?;
        self.touched_objects.push(k);
        self.gameobjects.get_mut(k)
    }

    pub fn add_creature(&mut self, c: Creature) -> Guid {
        let guid = c.guid;
        let cell = c.cell;
        if let Some(s) = c.spawn_guid {
            self.by_spawn.insert(s, guid);
        }
        // Its spawned events, and its summoner's hearing of it.
        let summoned = c.scripted.as_deref().is_some_and(|s| s.summoner.is_some());
        if event_ai::has_events_entry(&self.realm.data, c.entry)
            || self.instance.script.is_some()
            || summoned
        {
            self.ai_spawned.push(guid);
        }
        self.rare.note(&c);
        if awake(&c) {
            self.awake.insert(guid);
        }
        if !c.auras.is_empty() {
            self.with_auras.insert(guid);
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
        self.touched_objects.push(key);
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

    /// A creature leaves the map for good; its summoner hears of it, if it
    /// hasn't already (its corpse gone), as Thistle Tea's summons tell
    /// theirs as their process ends.
    pub fn remove_creature(&mut self, guid: Guid) -> Option<Creature> {
        let c = self.take_creature(guid)?;
        if !c.despawned
            && let Some(s) = c.scripted.as_deref().and_then(|s| s.summoner)
        {
            event_ai::summoned_despawned(self, s, guid, c.entry);
        }
        Some(c)
    }

    /// A creature out of the map, telling no one.
    fn take_creature(&mut self, guid: Guid) -> Option<Creature> {
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
            self.spells
                .dynamic
                .get_mut(&object)
                .map(|d| &mut d.observers)
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

    /// Players within `radius` yards of `at`, in three dimensions (Thistle
    /// Tea's `nearby_players`, mangos's `MessageDistDeliverer`).
    pub fn players_near(
        &self,
        at: wow_world_messages::vanilla::Vector3d,
        radius: f32,
    ) -> Vec<Guid> {
        let r2 = radius * radius;
        self.grid
            .around(at.x, at.y, radius)
            .filter(|g| guid::is_player(*g))
            .filter(|g| {
                self.players.get(g).is_some_and(|p| {
                    let d = p.position();
                    (d.x - at.x).powi(2) + (d.y - at.y).powi(2) + (d.z - at.z).powi(2) <= r2
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
