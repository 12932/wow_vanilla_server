pub mod aoi;
mod character_screen_handler;
pub mod command;
pub mod database;
pub mod update_object;
pub mod world_db;
#[allow(clippy::module_inception)]
mod world;
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

/// Configured target tick interval (10 Hz). The world prefers to run at
/// this cadence and `TickPacer` only departs from it under sustained
/// overload (see backoff/recovery rules below). Constants downstream that
/// need a tick-rate fallback (the bootstrap `tick_dt` on the very first
/// tick) read this rather than the live `TickPacer::current_interval`,
/// because they're computed before the pacer has any data.
pub const TARGET_INTERVAL: Duration = Duration::from_millis(100);

/// Floor on the adaptive tickrate (2 Hz). Once the pacer has backed off
/// this far, further slow-tick streaks are simply absorbed — going below
/// 2 Hz would make the simulation feel frozen to players.
const MAX_INTERVAL: Duration = Duration::from_millis(500);

const SAVE_INTERVAL: Duration = Duration::from_secs(60);

/// EMA coefficient on the "this tick was slow" indicator. α=0.2 is roughly
/// a 5-tick smoothing window — long enough that a single slow GC-pause-
/// looking tick doesn't trigger backoff, short enough that ~5 consecutive
/// slow ticks cross the threshold.
const SLOW_EMA_ALPHA: f32 = 0.2;

/// `slow_ema` threshold at which we double the interval. With α=0.2 this
/// corresponds to "the last ~5 ticks were predominantly slow".
const BACKOFF_THRESHOLD: f32 = 0.5;

/// We only count a tick as recovery-worthy "healthy" if it finished in
/// well under the current budget — naked equality at the boundary would
/// flap. 0.6 = 40 % headroom required.
const RECOVERY_HYSTERESIS: f32 = 0.6;

/// Consecutive headroom-meeting ticks required before we halve the
/// interval back toward `TARGET_INTERVAL`.
const RECOVERY_HEALTHY_STREAK: u32 = 30;

/// Adaptive tickrate controller. Owned by `run_world` and consulted once
/// per tick. Game logic must use wall-clock `dt`, never the pacer's
/// interval directly, because the interval changes at runtime.
struct TickPacer {
    target_interval: Duration,
    max_interval: Duration,
    current_interval: Duration,
    slow_ema: f32,
    healthy_streak: u32,
}

impl TickPacer {
    fn new(target_interval: Duration, max_interval: Duration) -> Self {
        Self {
            target_interval,
            max_interval,
            current_interval: target_interval,
            slow_ema: 0.0,
            healthy_streak: 0,
        }
    }

    /// Feed the most recent tick's measured duration. Updates internal
    /// state, possibly transitions to a new `current_interval`, and
    /// returns how long the caller should sleep until the next tick
    /// (`current_interval - tick_duration`, clamped at zero).
    fn observe(&mut self, tick_duration: Duration) -> Duration {
        let slow_now = if tick_duration > self.current_interval {
            1.0
        } else {
            0.0
        };
        self.slow_ema = SLOW_EMA_ALPHA * slow_now + (1.0 - SLOW_EMA_ALPHA) * self.slow_ema;

        if self.slow_ema > BACKOFF_THRESHOLD && self.current_interval < self.max_interval {
            let new_interval = (self.current_interval * 2).min(self.max_interval);
            info!(
                "tickrate backoff: now {} ms ({:.1} Hz)",
                new_interval.as_millis(),
                1.0 / new_interval.as_secs_f32()
            );
            self.current_interval = new_interval;
            self.slow_ema = 0.0;
            self.healthy_streak = 0;
        } else {
            let headroom = self.current_interval.mul_f32(RECOVERY_HYSTERESIS);
            if tick_duration <= headroom {
                self.healthy_streak = self.healthy_streak.saturating_add(1);
            } else if slow_now > 0.0 {
                self.healthy_streak = 0;
            }
            if self.healthy_streak >= RECOVERY_HEALTHY_STREAK
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
            }
        }

        self.current_interval.saturating_sub(tick_duration)
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

    let mut world = World::with_creatures(clients_waiting_to_join, creatures);

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

    let mut next_save = Instant::now() + SAVE_INTERVAL;
    let mut pacer = TickPacer::new(TARGET_INTERVAL, MAX_INTERVAL);

    loop {
        let before = Instant::now();

        world.tick(&mut db, pacer.current_interval).await;

        let after = Instant::now();
        let tick_duration = after.duration_since(before);

        let final_save = shutdown.load(Ordering::SeqCst);
        if final_save || after >= next_save {
            world.sync_clients_to_db(&mut db);
            // Worlddb is authoritative for creatures — skip them in snapshot.
            let snap = WorldSnapshot::capture(&db, &slab::Slab::new());
            match snap.save(SNAPSHOT_PATH) {
                Ok(()) => tracing::debug!("Snapshot saved to {SNAPSHOT_PATH}"),
                Err(e) => warn!("Snapshot save failed: {e}"),
            }
            // Return excess slab / hashmap capacity after long-running churn.
            // This is outside the tick hot path so the cost is fine.
            world.shrink_periodic();
            next_save = after + SAVE_INTERVAL;
        }

        if final_save {
            info!("Shutdown complete");
            std::process::exit(0);
        }

        let sleep_for = pacer.observe(tick_duration);
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
        // 100 slow ticks (200 ms each, well over the 100 ms target). EMA must
        // cross BACKOFF_THRESHOLD, the interval doubles to 200 ms, and
        // continuing slow ticks at 200 ms then push us to 400 ms, then 500 ms
        // (capped).
        for _ in 0..100 {
            p.observe(Duration::from_millis(600));
        }
        assert_eq!(p.current_interval, MAX_INTERVAL);
    }

    #[test]
    fn healthy_streak_triggers_recovery() {
        let mut p = pacer();
        // Force interval up to MAX_INTERVAL.
        for _ in 0..100 {
            p.observe(Duration::from_millis(600));
        }
        assert_eq!(p.current_interval, MAX_INTERVAL);

        // Now feed healthy ticks (well below the 500 ms * 0.6 = 300 ms
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
            p.observe(Duration::from_secs(5));
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
        let sleep_for = p.observe(Duration::from_millis(250));
        assert_eq!(sleep_for, Duration::ZERO);
    }
}
