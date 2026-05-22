pub mod aoi;
mod character_screen_handler;
pub mod command;
pub mod database;
pub mod net_stats;
pub mod cell;
pub mod update_object;
pub mod world_db;
#[allow(clippy::module_inception)]
pub mod world;
pub mod world_opcode_handler;

use crate::snapshot::{WorldSnapshot, SNAPSHOT_PATH};
use crate::world::database::WorldDatabase;
use crate::world::world::World;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::sleep;
use tracing::{debug, info, warn};
use world::client::character_screen_client::CharacterScreenClient;
use wow_srp::normalized_string::NormalizedString;
use wow_srp::vanilla_header::ProofSeed;
use wow_world_messages::vanilla::tokio_expect_client_message;
use wow_world_messages::vanilla::*;

pub async fn world(users: crate::auth::UserCache) {
    let listener = TcpListener::bind("0.0.0.0:8085").await.unwrap();
    info!("world: listening on 0.0.0.0:8085");
    let (world, clients_waiting_to_join) = mpsc::channel(32);

    tokio::spawn(run_world(clients_waiting_to_join));

    loop {
        let (stream, peer) = listener.accept().await.unwrap();
        debug!("world: accepted connection from {peer}");

        tokio::spawn(character_screen(stream, users.clone(), world.clone()));
    }
}

/// Test-only defaults for `TickPacer` matching the config defaults in
/// `[tick]` of `config.toml`. Production reads them from the config file
/// via `TickPacer::new_from_config`; these are referenced only by the
/// in-file `pacer_tests` module.
#[cfg(test)]
const TARGET_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(test)]
const MAX_INTERVAL: Duration = Duration::from_millis(1000);

/// Adaptive tickrate controller. Owned by `run_world` and consulted once
/// per tick. Game logic must use wall-clock `dt`, never the pacer's
/// interval directly, because the interval changes at runtime.
///
/// Stage 4: this struct also lives per-cell (one `TickPacer` per
/// `CellState`) so `.cells` can show the per-cell adaptive
/// state — `current_interval`, `slow_ema` — independent of the global
/// pacer in `run_world`.
#[derive(Debug, Clone)]
pub(crate) struct TickPacer {
    pub(crate) target_interval: Duration,
    pub(crate) max_interval: Duration,
    pub(crate) current_interval: Duration,
    pub(crate) slow_ema: f32,
    pub(crate) healthy_streak: u32,
    slow_ema_alpha: f32,
    backoff_threshold: f32,
    recovery_hysteresis: f32,
    recovery_healthy_streak: u32,
}

/// Returned by `TickPacer::observe` when the tick triggered a transition
/// between rates. `None` (in the option) means this tick was business as
/// usual. The caller — `run_world` — uses this to broadcast an in-game
/// system message so operators watching the world can see adaptive
/// pacing kick in without tailing the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickRateChange {
    Backoff { new_interval: Duration },
    Recovery { new_interval: Duration },
}

impl TickPacer {
    /// Test-only constructor that fills the pacing thresholds with the
    /// canonical config defaults. Production should use
    /// [`Self::new_from_config`].
    #[cfg(test)]
    fn new(target_interval: Duration, max_interval: Duration) -> Self {
        let cfg = crate::config::TickConfig::default();
        Self {
            target_interval,
            max_interval,
            current_interval: target_interval,
            slow_ema: 0.0,
            healthy_streak: 0,
            slow_ema_alpha: cfg.slow_ema_alpha,
            backoff_threshold: cfg.backoff_threshold,
            recovery_hysteresis: cfg.recovery_hysteresis,
            recovery_healthy_streak: cfg.recovery_healthy_streak,
        }
    }

    /// Build a pacer from the global config's `[tick]` section. Snapshots
    /// the thresholds at construction time — no hot reload, see config
    /// module docs.
    pub(crate) fn new_from_config(cfg: &crate::config::TickConfig) -> Self {
        Self {
            target_interval: cfg.target_interval(),
            max_interval: cfg.max_interval(),
            current_interval: cfg.target_interval(),
            slow_ema: 0.0,
            healthy_streak: 0,
            slow_ema_alpha: cfg.slow_ema_alpha,
            backoff_threshold: cfg.backoff_threshold,
            recovery_hysteresis: cfg.recovery_hysteresis,
            recovery_healthy_streak: cfg.recovery_healthy_streak,
        }
    }

    /// Feed the most recent tick's measured duration. Updates internal
    /// state, possibly transitions to a new `current_interval`, and
    /// returns `(sleep_for, change)` — how long the caller should sleep
    /// until the next tick (`current_interval - tick_duration`, clamped
    /// at zero) and an optional rate-change event so the caller can
    /// surface it (in-game broadcast, metrics, etc.).
    pub(crate) fn observe(&mut self, tick_duration: Duration) -> (Duration, Option<TickRateChange>) {
        let slow_now = if tick_duration > self.current_interval {
            1.0
        } else {
            0.0
        };
        self.slow_ema =
            self.slow_ema_alpha * slow_now + (1.0 - self.slow_ema_alpha) * self.slow_ema;

        let mut change: Option<TickRateChange> = None;
        if self.slow_ema > self.backoff_threshold && self.current_interval < self.max_interval {
            let new_interval = (self.current_interval * 2).min(self.max_interval);
            info!(
                "tickrate backoff: now {} ms ({:.1} Hz)",
                new_interval.as_millis(),
                1.0 / new_interval.as_secs_f32()
            );
            self.current_interval = new_interval;
            self.slow_ema = 0.0;
            self.healthy_streak = 0;
            change = Some(TickRateChange::Backoff { new_interval });
        } else {
            let headroom = self.current_interval.mul_f32(self.recovery_hysteresis);
            if tick_duration <= headroom {
                self.healthy_streak = self.healthy_streak.saturating_add(1);
            } else if slow_now > 0.0 {
                self.healthy_streak = 0;
            }
            if self.healthy_streak >= self.recovery_healthy_streak
                && self.current_interval > self.target_interval
            {
                let new_interval = (self.current_interval / 2).max(self.target_interval);
                info!(
                    "tickrate recovery: now {} ms ({:.1} Hz)",
                    new_interval.as_millis(),
                    1.0 / new_interval.as_secs_f32()
                );
                self.current_interval = new_interval;
                self.healthy_streak = 0;
                change = Some(TickRateChange::Recovery { new_interval });
            }
        }

        (
            self.current_interval.saturating_sub(tick_duration),
            change,
        )
    }
}

async fn run_world(clients_waiting_to_join: mpsc::Receiver<CharacterScreenClient>) {
    let mut db = match WorldSnapshot::load(SNAPSHOT_PATH) {
        Ok(Some(snap)) => {
            info!("Restoring characters from {SNAPSHOT_PATH}");
            snap.restore_db_only()
        }
        Ok(None) => {
            info!("No snapshot found; starting fresh");
            WorldDatabase::new()
        }
        Err(e) => {
            warn!("Failed to load {SNAPSHOT_PATH}: {e}; starting fresh");
            WorldDatabase::new()
        }
    };

    let creatures = match std::env::var("WOW_VANILLA_WORLDDB") {
        Ok(path) => match crate::world::world_db::load_creatures(&path) {
            Ok(slab) => slab,
            Err(e) => {
                warn!("worlddb load from '{path}' failed: {e}; starting with empty world");
                slab::Slab::new()
            }
        },
        Err(_) => {
            info!("WOW_VANILLA_WORLDDB unset; spawning legacy test creature only");
            let mut s = slab::Slab::new();
            s.insert(
                crate::world::world_opcode_handler::creature::Creature::new(
                    "Thing",
                    db.new_guid().into(),
                ),
            );
            s
        }
    };

    let mut world =
        World::with_creatures_and_db(clients_waiting_to_join, creatures, db);

    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                info!("Ctrl-C received; world will save and exit after current tick");
                shutdown.store(true, Ordering::SeqCst);
            }
        });
    }

    let tick_cfg = &crate::config::config().tick;
    let save_interval = tick_cfg.save_interval();
    let mut next_save = Instant::now() + save_interval;
    let mut pacer = TickPacer::new_from_config(tick_cfg);

    loop {
        let before = Instant::now();

        world.tick(pacer.current_interval).await;

        let after = Instant::now();
        let tick_duration = after.duration_since(before);

        let final_save = shutdown.load(Ordering::SeqCst);
        if final_save || after >= next_save {
            world.sync_clients_to_db().await;
            // Worlddb is authoritative for creatures — skip them in snapshot.
            let db = world.db.lock().await;
            let snap = WorldSnapshot::capture(&db, &slab::Slab::new());
            drop(db);
            match snap.save(SNAPSHOT_PATH) {
                Ok(()) => tracing::debug!("Snapshot saved to {SNAPSHOT_PATH}"),
                Err(e) => warn!("Snapshot save failed: {e}"),
            }
            // Return excess slab / hashmap capacity after long-running churn.
            // This is outside the tick hot path so the cost is fine.
            world.shrink_periodic().await;
            next_save = after + save_interval;
        }

        if final_save {
            info!("Shutdown complete");
            std::process::exit(0);
        }

        let (sleep_for, change) = pacer.observe(tick_duration);
        // The GLOBAL pacer transition is no longer chat-broadcast to
        // every player — per-cell pacers now emit their own
        // cell-scoped messages (only to players in the affected
        // cell). The orchestrator-level rate is just logged so the
        // operator can see it via `RUST_LOG=info` / tracing.
        if let Some(change) = change {
            match change {
                TickRateChange::Backoff { new_interval } => info!(
                    "global tickrate backoff: {} ms ({:.1} Hz)",
                    new_interval.as_millis(),
                    1.0 / new_interval.as_secs_f32()
                ),
                TickRateChange::Recovery { new_interval } => info!(
                    "global tickrate recovery: {} ms ({:.1} Hz)",
                    new_interval.as_millis(),
                    1.0 / new_interval.as_secs_f32()
                ),
            }
        }
        if !sleep_for.is_zero() {
            sleep(sleep_for).await;
        }
    }
}

async fn character_screen(
    stream: TcpStream,
    users: crate::auth::UserCache,
    world: Sender<CharacterScreenClient>,
) {
    if let Err(e) = character_screen_inner(stream, users, world).await {
        // Per-connection errors are routine: clients disconnect mid-handshake,
        // send garbage, race the auth registration. Log and drop the socket.
        tracing::debug!("character_screen handshake aborted: {e}");
    }
}

async fn character_screen_inner(
    mut stream: TcpStream,
    users: crate::auth::UserCache,
    world: Sender<CharacterScreenClient>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let seed = ProofSeed::new();

    SMSG_AUTH_CHALLENGE {
        server_seed: seed.seed(),
    }
    .tokio_write_unencrypted_server(&mut stream)
    .await?;

    let c = tokio_expect_client_message::<CMSG_AUTH_SESSION, _>(&mut stream).await?;
    let account_name = c.username;

    let session_key = {
        let mut server = users
            .lock()
            .map_err(|_| "users mutex poisoned".to_string())?;
        let Some(srp) = server.get_mut(&account_name) else {
            return Err(format!("unknown account '{account_name}'").into());
        };
        *srp.session_key()
    };

    let mut encryption = seed
        .into_server_header_crypto(
            &NormalizedString::new(&account_name)?,
            session_key,
            c.client_proof,
            c.client_seed,
        )
        .map_err(|e| format!("SRP handshake failed for '{account_name}': {e:?}"))?;

    SMSG_AUTH_RESPONSE {
        result: SMSG_AUTH_RESPONSE_WorldResult::AuthOk {
            billing_flags: 0,
            billing_rested: 0,
            billing_time: 0,
        },
    }
    .tokio_write_encrypted_server(&mut stream, encryption.encrypter())
    .await?;

    world
        .send(CharacterScreenClient::new(account_name, stream, encryption))
        .await
        .map_err(|e| format!("world receiver dropped: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod pacer_tests {
    use super::*;

    fn pacer() -> TickPacer {
        TickPacer::new(TARGET_INTERVAL, MAX_INTERVAL)
    }

    #[test]
    fn slow_streak_triggers_backoff() {
        let mut p = pacer();
        // 100 slow ticks (600 ms each, well over the 100 ms target). EMA must
        // cross BACKOFF_THRESHOLD repeatedly: the interval doubles
        // 100→200→400→800 and then saturates at MAX_INTERVAL (1000 ms,
        // 1 Hz floor) on the next attempt.
        for _ in 0..100 {
            p.observe(Duration::from_millis(1200));
        }
        assert_eq!(p.current_interval, MAX_INTERVAL);
    }

    #[test]
    fn healthy_streak_triggers_recovery() {
        let mut p = pacer();
        // Force interval up to MAX_INTERVAL.
        for _ in 0..100 {
            p.observe(Duration::from_millis(1200));
        }
        assert_eq!(p.current_interval, MAX_INTERVAL);

        // Now feed healthy ticks (well below the 1000 ms * 0.6 = 600 ms
        // hysteresis threshold). After enough streaks the interval halves
        // repeatedly back down to TARGET_INTERVAL.
        for _ in 0..1000 {
            p.observe(Duration::from_millis(50));
        }
        assert_eq!(p.current_interval, TARGET_INTERVAL);
    }

    #[test]
    fn cap_at_target() {
        let mut p = pacer();
        for _ in 0..10_000 {
            p.observe(Duration::from_micros(1));
        }
        assert_eq!(p.current_interval, TARGET_INTERVAL);
    }

    #[test]
    fn cap_at_max() {
        let mut p = pacer();
        for _ in 0..10_000 {
            p.observe(Duration::from_secs(10));
        }
        assert_eq!(p.current_interval, MAX_INTERVAL);
    }

    #[test]
    fn mixed_pattern_holds_steady() {
        let mut p = pacer();
        // Alternating "right-at-budget" and "well-under" ticks. Neither
        // sustained-slow (EMA stays below 0.5) nor sustained-headroom-clear
        // (the at-budget ones break the recovery streak). Interval should
        // remain at TARGET_INTERVAL the entire run.
        for i in 0..200 {
            let d = if i % 2 == 0 {
                Duration::from_millis(95)
            } else {
                Duration::from_millis(30)
            };
            p.observe(d);
            assert_eq!(p.current_interval, TARGET_INTERVAL);
        }
    }

    #[test]
    fn sleep_duration_clamps_at_zero() {
        let mut p = pacer();
        // A tick that overran the budget produces zero sleep (not a panic
        // from Duration underflow).
        let (sleep_for, _) = p.observe(Duration::from_millis(250));
        assert_eq!(sleep_for, Duration::ZERO);
    }

    #[test]
    fn observe_reports_backoff_transition_once() {
        let mut p = pacer();
        let mut backoffs = 0;
        for _ in 0..40 {
            if let (_, Some(TickRateChange::Backoff { .. })) =
                p.observe(Duration::from_millis(1200))
            {
                backoffs += 1;
            }
        }
        // Should hit Backoff exactly four times —
        // 100→200→400→800→1000 ms (capped at MAX_INTERVAL on the
        // fourth). After saturating, no further transitions emit.
        assert_eq!(backoffs, 4);
        assert_eq!(p.current_interval, MAX_INTERVAL);
    }

    #[test]
    fn observe_reports_recovery_transition() {
        let mut p = pacer();
        // Force backoff first.
        for _ in 0..100 {
            p.observe(Duration::from_millis(1200));
        }
        assert_eq!(p.current_interval, MAX_INTERVAL);
        // Now drip-feed healthy ticks and confirm at least one Recovery
        // event surfaces on the way back to target.
        let mut recoveries = 0;
        for _ in 0..1000 {
            if let (_, Some(TickRateChange::Recovery { .. })) =
                p.observe(Duration::from_millis(50))
            {
                recoveries += 1;
            }
        }
        assert!(recoveries >= 1, "expected at least one Recovery event");
        assert_eq!(p.current_interval, TARGET_INTERVAL);
    }
}
