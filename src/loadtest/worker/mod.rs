//! Worker process: hosts N simulated bots. Runs standalone or under an orchestrator.

pub mod auth;
pub mod bot;
pub mod metrics;
pub mod movement;
pub mod pvp;
pub mod world;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slab::Slab;
use tokio::net::TcpStream;
use tracing::{info, warn};

use crate::protocol::{ToOrchestrator, ToWorker, read_frame, write_frame};
use crate::worker::bot::{BotConfig, BotExit, BotHandle};
use crate::worker::metrics::Metrics;

/// How long `stop` and `drain` wait for bots to finish before aborting them.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// Behavioral mode for every bot spawned by a worker. Picked by the
/// `--mode` CLI flag at startup; can't be mixed within one worker
/// (orchestrator-driven mid-flight mode swaps are not supported).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BotMode {
    /// Random-walk inside a 60 yd box around the spawn anchor.
    Random,
    /// PvP arena fight in Gurubashi Pit.
    Pvp,
    /// Amazing Race: each bot self-teleports to Booty Bay and runs the
    /// navmesh path to Gurubashi Arena (and back, in a loop).
    Race,
}

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub worker_id: String,
    pub auth_addr: String,
    pub world_addr_override: Option<String>,
    pub username_prefix: String,
    /// Number of bots to spawn at startup (used both in standalone and as the
    /// initial population when running under an orchestrator).
    pub initial_clients: u32,
    /// Spread each spawn batch over this many seconds. 0 falls back to a
    /// fixed 5 ms inter-bot delay.
    pub ramp_up_secs: u32,
    /// Optional orchestrator endpoint. None → standalone mode.
    pub orchestrator: Option<String>,
    /// Per-bot behavior. See [`BotMode`].
    pub mode: BotMode,
    /// The first account slot this worker uses. Workers sharing a server
    /// need disjoint slot ranges, or their bots share accounts.
    pub slot_offset: u32,
    /// See [`BotConfig::password`].
    pub password: Option<String>,
    /// See [`BotConfig::login_timeout`].
    pub login_timeout: Duration,
}

/// The worker's bots, and the account slots they use.
pub struct Fleet {
    bots: Mutex<Slab<BotHandle>>,
    slots: Mutex<Slots>,
    next_id: AtomicU64,
}

/// Account slots: the next never-used one, and the ones bots have freed,
/// lowest first, so a stop and respawn reuses accounts instead of walking
/// past a server's seeded range.
struct Slots {
    next: u32,
    free: BTreeSet<u32>,
}

impl Fleet {
    pub fn new(slot_offset: u32) -> Arc<Self> {
        Arc::new(Self {
            bots: Mutex::new(Slab::new()),
            slots: Mutex::new(Slots {
                next: slot_offset,
                free: BTreeSet::new(),
            }),
            next_id: AtomicU64::new(0),
        })
    }

    fn take_slot(&self) -> u32 {
        let mut slots = self.slots.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(slot) = slots.free.pop_first() {
            return slot;
        }
        let slot = slots.next;
        slots.next = slots.next.saturating_add(1);
        slot
    }

    /// Start one bot.
    fn spawn(self: &Arc<Self>, cfg: &BotConfig, metrics: &Arc<Metrics>) {
        let slot = self.take_slot();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        // Hold the slab while spawning, so a bot that ends at once finds its
        // own entry when it releases it.
        let mut bots = self.bots.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = bots.vacant_entry();
        let exit = BotExit {
            fleet: self.clone(),
            key: entry.key(),
            id,
            slot,
            metrics: metrics.clone(),
            alive: false,
        };
        entry.insert(bot::spawn(slot, cfg.clone(), metrics.clone(), exit));
    }

    /// A bot has ended: free its slot, and its slab entry if it is still
    /// there (`stop` and `drain` take entries out themselves).
    pub fn release(&self, key: usize, id: u64, slot: u32) {
        {
            let mut bots = self.bots.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if bots.get(key).is_some_and(|h| h.id == id) {
                bots.remove(key);
            }
        }
        self.slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .free
            .insert(slot);
    }

    /// Take up to `count` bots out of the slab (all with `None`).
    fn take(&self, count: Option<usize>) -> Vec<BotHandle> {
        let mut bots = self.bots.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let keys: Vec<usize> = bots
            .iter()
            .map(|(k, _)| k)
            .take(count.unwrap_or(usize::MAX))
            .collect();
        keys.into_iter().map(|k| bots.remove(k)).collect()
    }
}

/// Tell `handles` to stop, give them [`STOP_GRACE`] between them to finish,
/// then abort the rest. An aborted bot still frees its slot: its exit guard
/// drops with its future.
async fn stop_all(handles: Vec<BotHandle>) {
    for h in &handles {
        h.shutdown.notify_one();
    }
    let deadline = tokio::time::Instant::now() + STOP_GRACE;
    for mut h in handles {
        if tokio::time::timeout_at(deadline, &mut h.join).await.is_err() {
            h.join.abort();
        }
    }
}

pub async fn run(cfg: WorkerConfig) -> std::io::Result<()> {
    let metrics = Arc::new(Metrics::default());
    let fleet = Fleet::new(cfg.slot_offset);

    // Worker-wide latch consulted by PvP-mode bots to decide whether to
    // gather or fight. Created here so every bot we spawn (initial batch
    // and any orchestrator-driven follow-ups) shares the same handle.
    // We flip it to `true` after the initial spawn completes, signalling
    // "everyone you were going to spawn with is up — start fighting".
    let battle_started = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Race path + shared VanillaMap. Shared across every Race-mode
    // bot in this worker. Built once at worker startup. The drivers read
    // heights from the map through a shared reference, to snap Z to the
    // ground every heartbeat (the path is sparse; Z comes from per-tick
    // height lookups).
    //
    // Without maps configured, bots run the hand-picked
    // `hardcoded_bb_to_arena` waypoints with no Z sampling: they clip
    // through scenery, but the load is the same from the server's side.
    // With maps configured, a path that can't be built is an error: the
    // race would otherwise run on a made-up route without saying so.
    let (race_path, race_map): (
        Arc<[wow_world_messages::vanilla::Vector3d]>,
        Option<Arc<rustigator::vanilla::VanillaMap>>,
    ) = if cfg.mode == BotMode::Race {
        match bot::race::build_race_path() {
            Ok(bot::race::RacePath::Navmesh(path, map)) => (path.into(), Some(map)),
            Ok(bot::race::RacePath::NoMaps) => {
                tracing::warn!(
                    "built without WOW_VANILLA_USE_MAPS: racing the hard-coded BB→Arena polyline. \
                     The geometry is coarse and Z won't track the ground."
                );
                (bot::race::hardcoded_bb_to_arena().into(), None)
            }
            Err(e) => {
                return Err(std::io::Error::other(format!("race path: {e}")));
            }
        }
    } else {
        (Arc::from(Vec::new()), None)
    };

    let bot_cfg = BotConfig {
        auth_addr: cfg.auth_addr.clone(),
        world_addr_override: cfg.world_addr_override.clone(),
        username_prefix: cfg.username_prefix.clone(),
        password: cfg.password.clone(),
        login_timeout: cfg.login_timeout,
        mode: cfg.mode,
        battle_started: battle_started.clone(),
        race_path: race_path.clone(),
        race_map: race_map.clone(),
    };

    if cfg.initial_clients > 0 {
        spawn_n(
            &fleet,
            &bot_cfg,
            &metrics,
            cfg.initial_clients,
            cfg.ramp_up_secs,
        )
        .await;
        if cfg.mode == BotMode::Pvp {
            tracing::info!(
                "Initial PvP spawn batch complete ({} bots); signalling battle start",
                cfg.initial_clients
            );
            battle_started.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    // Local metrics printer — runs in both standalone and orchestrator modes.
    // Prints a single human-readable line per second so an operator watching
    // the terminal can see ramp progress at a glance.
    let local_metrics = metrics.clone();
    let local_worker_id = cfg.worker_id.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut prev_in = 0u64;
        let mut prev_out = 0u64;
        let mut prev_moves = 0u64;
        let started = std::time::Instant::now();
        loop {
            tick.tick().await;
            let s = local_metrics.snapshot();
            let target = local_metrics
                .target_bots
                .load(std::sync::atomic::Ordering::Relaxed);
            let in_per_s = s.messages_in.saturating_sub(prev_in);
            let out_per_s = s.messages_out.saturating_sub(prev_out);
            let moves_per_s = s.movement_in.saturating_sub(prev_moves);
            prev_in = s.messages_in;
            prev_out = s.messages_out;
            prev_moves = s.movement_in;
            info!(
                "[{wid}] t={t}s | alive {alive}/{target} | auth {a_ok}ok/{a_fail}fail | world {w_ok}ok/{w_fail}fail | login timeouts {lt} | msgs in/s {in_s} out/s {out_s} | others' moves in/s {mv_s} | skipped {sk} parse_err {pe} | send_err {se}",
                wid = local_worker_id,
                t = started.elapsed().as_secs(),
                alive = s.bots_alive,
                target = target,
                a_ok = s.auth_ok,
                a_fail = s.auth_fail,
                w_ok = s.world_ok,
                w_fail = s.world_fail,
                lt = s.login_timeouts,
                sk = s.skipped_in,
                pe = s.parse_errors,
                mv_s = moves_per_s,
                in_s = in_per_s,
                out_s = out_per_s,
                se = s.send_errors,
            );
        }
    });

    if let Some(orch_addr) = cfg.orchestrator.clone() {
        run_under_orchestrator(orch_addr, cfg, fleet, bot_cfg, metrics).await
    } else {
        // Standalone: park forever until Ctrl-C.
        tokio::signal::ctrl_c().await.ok();
        info!("Ctrl-C received; draining bots");
        stop_all(fleet.take(None)).await;
        Ok(())
    }
}

async fn run_under_orchestrator(
    orch_addr: String,
    cfg: WorkerConfig,
    fleet: Arc<Fleet>,
    bot_cfg: BotConfig,
    metrics: Arc<Metrics>,
) -> std::io::Result<()> {
    loop {
        let stream = match TcpStream::connect(&orch_addr).await {
            Ok(s) => s,
            Err(e) => {
                warn!("worker: orchestrator connect failed: {e}; retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        let (mut r, mut w) = stream.into_split();

        let hello = ToOrchestrator::Hello {
            worker_id: cfg.worker_id.clone(),
            started_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        if let Err(e) = write_frame(&mut w, &hello).await {
            warn!("worker: hello failed: {e}; reconnecting in 5s");
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        // Spawn a metrics-pusher.
        let push_metrics = metrics.clone();
        let push_handle = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let msg = ToOrchestrator::Metrics(push_metrics.snapshot());
                if write_frame(&mut w, &msg).await.is_err() {
                    return;
                }
            }
        });

        // Read commands until the orchestrator goes away.
        loop {
            let cmd: ToWorker = match read_frame(&mut r).await {
                Ok(c) => c,
                Err(e) => {
                    warn!("worker: orchestrator read error: {e}; reconnecting");
                    break;
                }
            };
            match cmd {
                ToWorker::Spawn { count } => {
                    spawn_n(&fleet, &bot_cfg, &metrics, count, cfg.ramp_up_secs).await;
                }
                ToWorker::Stop { count } => {
                    let handles = fleet.take(Some(count as usize));
                    metrics.target_bots.fetch_sub(
                        u32::try_from(handles.len()).unwrap_or(u32::MAX),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    // Stop in the background, so commands keep flowing.
                    tokio::spawn(stop_all(handles));
                }
                ToWorker::Drain => {
                    stop_all(fleet.take(None)).await;
                    return Ok(());
                }
            }
        }

        push_handle.abort();
        // Loop reconnect.
    }
}

async fn spawn_n(
    fleet: &Arc<Fleet>,
    bot_cfg: &BotConfig,
    metrics: &Arc<Metrics>,
    count: u32,
    ramp_up_secs: u32,
) {
    // Pick per-bot delay. With `ramp_up_secs > 0`, spread the batch evenly
    // across that window. With `ramp_up_secs == 0` we sleep zero between
    // spawns — caller asked for max-rate, give them max-rate.
    let stagger = if ramp_up_secs > 0 && count > 0 {
        Duration::from_millis((u64::from(ramp_up_secs) * 1000) / u64::from(count))
    } else {
        Duration::ZERO
    };
    info!(
        "spawning {count} bots over ~{:.1}s ({}ms between bots)",
        stagger.as_millis() as f64 * count as f64 / 1000.0,
        stagger.as_millis()
    );
    metrics
        .target_bots
        .fetch_add(count, std::sync::atomic::Ordering::Relaxed);

    // Emit ~10 progress lines per spawn batch so the operator can see the
    // ramp progressing without waiting for the per-second metrics tick.
    // Clamp the step to at least 1 so very small batches (count=1) still
    // log once at completion.
    let progress_step = (count / 10).max(1);
    let start = std::time::Instant::now();

    for i in 0..count {
        fleet.spawn(bot_cfg, metrics);

        let spawned = i + 1;
        if spawned % progress_step == 0 || spawned == count {
            let m = metrics.snapshot();
            info!(
                "spawn progress: {spawned}/{count} requested ({:.0}%) | auth ok {ok} fail {fail} | world ok {wok} | t={t:.1}s",
                100.0 * spawned as f64 / count as f64,
                ok = m.auth_ok,
                fail = m.auth_fail,
                wok = m.world_ok,
                t = start.elapsed().as_secs_f64(),
            );
        }

        tokio::time::sleep(stagger).await;
    }
}
