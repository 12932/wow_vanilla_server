//! Worker process: hosts N simulated bots. Runs standalone or under an orchestrator.

pub mod auth;
pub mod bot;
pub mod metrics;
pub mod movement;
pub mod world;

use std::sync::Arc;
use std::time::Duration;

use slab::Slab;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::protocol::{ToOrchestrator, ToWorker, read_frame, write_frame};
use crate::worker::bot::{BotConfig, BotHandle, spawn};
use crate::worker::metrics::Metrics;

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
}

pub async fn run(cfg: WorkerConfig) -> std::io::Result<()> {
    let metrics = Arc::new(Metrics::default());
    let bots: Arc<Mutex<Slab<BotHandle>>> = Arc::new(Mutex::new(Slab::new()));
    let next_slot = Arc::new(std::sync::atomic::AtomicU32::new(0));

    let bot_cfg = BotConfig {
        auth_addr: cfg.auth_addr.clone(),
        world_addr_override: cfg.world_addr_override.clone(),
        username_prefix: cfg.username_prefix.clone(),
    };

    if cfg.initial_clients > 0 {
        spawn_n(
            &bots,
            &next_slot,
            &bot_cfg,
            &metrics,
            cfg.initial_clients,
            cfg.ramp_up_secs,
        )
        .await;
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
        let started = std::time::Instant::now();
        loop {
            tick.tick().await;
            let s = local_metrics.snapshot();
            let target = local_metrics
                .target_bots
                .load(std::sync::atomic::Ordering::Relaxed);
            let in_per_s = s.messages_in.saturating_sub(prev_in);
            let out_per_s = s.messages_out.saturating_sub(prev_out);
            prev_in = s.messages_in;
            prev_out = s.messages_out;
            info!(
                "[{wid}] t={t}s | alive {alive}/{target} | auth {a_ok}ok/{a_fail}fail | world {w_ok}ok/{w_fail}fail | msgs in/s {in_s} out/s {out_s} | send_err {se}",
                wid = local_worker_id,
                t = started.elapsed().as_secs(),
                alive = s.bots_alive,
                target = target,
                a_ok = s.auth_ok,
                a_fail = s.auth_fail,
                w_ok = s.world_ok,
                w_fail = s.world_fail,
                in_s = in_per_s,
                out_s = out_per_s,
                se = s.send_errors,
            );
        }
    });

    if let Some(orch_addr) = cfg.orchestrator.clone() {
        run_under_orchestrator(orch_addr, cfg, bots, next_slot, bot_cfg, metrics).await
    } else {
        // Standalone: park forever until Ctrl-C.
        tokio::signal::ctrl_c().await.ok();
        info!("Ctrl-C received; draining bots");
        drain_all(&bots).await;
        Ok(())
    }
}

async fn run_under_orchestrator(
    orch_addr: String,
    cfg: WorkerConfig,
    bots: Arc<Mutex<Slab<BotHandle>>>,
    next_slot: Arc<std::sync::atomic::AtomicU32>,
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
                    spawn_n(&bots, &next_slot, &bot_cfg, &metrics, count, cfg.ramp_up_secs)
                        .await;
                }
                ToWorker::Stop { count } => {
                    stop_n(&bots, count).await;
                }
                ToWorker::Drain => {
                    drain_all(&bots).await;
                    return Ok(());
                }
            }
        }

        push_handle.abort();
        // Loop reconnect.
    }
}

async fn spawn_n(
    bots: &Arc<Mutex<Slab<BotHandle>>>,
    next_slot: &Arc<std::sync::atomic::AtomicU32>,
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
    for _ in 0..count {
        let slot = next_slot.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let handle = spawn(slot, bot_cfg.clone(), metrics.clone());
        bots.lock().await.insert(handle);
        tokio::time::sleep(stagger).await;
    }
}

async fn stop_n(bots: &Arc<Mutex<Slab<BotHandle>>>, count: u32) {
    let mut guard = bots.lock().await;
    let mut keys: Vec<usize> = guard.iter().map(|(k, _)| k).collect();
    keys.truncate(count as usize);
    for k in keys {
        if let Some(h) = guard.try_remove(k) {
            h.shutdown.notify_one();
            // Detach the join handle — bot will exit shortly on its own.
            drop(h.join);
        }
    }
}

async fn drain_all(bots: &Arc<Mutex<Slab<BotHandle>>>) {
    let handles: Vec<BotHandle> = {
        let mut guard = bots.lock().await;
        guard.drain().collect()
    };
    for h in &handles {
        h.shutdown.notify_one();
    }
    for h in handles {
        let _ = tokio::time::timeout(Duration::from_secs(5), h.join).await;
    }
}
