//! Single bot lifecycle: auth → world handshake → walk → shutdown.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use wow_world_messages::vanilla::ClientMessage as _;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::CMSG_PING;

use crate::worker::auth;
use crate::worker::metrics::Metrics;
use crate::worker::movement::MovementDriver;
use crate::worker::world;

#[derive(Debug, Clone)]
pub struct BotConfig {
    pub auth_addr: String,
    /// Override the realm address returned by the auth server. Useful when the
    /// auth server tells us `vpn.gtker.com:8085` but we actually want `127.0.0.1:8085`.
    pub world_addr_override: Option<String>,
    pub username_prefix: String,
}

pub struct BotHandle {
    #[allow(dead_code)] // useful for debugging; kept on the handle even when unused
    pub username: String,
    pub shutdown: Arc<Notify>,
    pub join: JoinHandle<()>,
}

pub fn spawn(slot: u32, cfg: BotConfig, metrics: Arc<Metrics>) -> BotHandle {
    let username = format!("{}{:04}", cfg.username_prefix.to_uppercase(), slot);
    let shutdown = Arc::new(Notify::new());
    let join = {
        let username = username.clone();
        let cfg = cfg.clone();
        let metrics = metrics.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            run_bot(slot, username, cfg, metrics, shutdown).await;
        })
    };
    BotHandle {
        username,
        shutdown,
        join,
    }
}

async fn run_bot(
    slot: u32,
    username: String,
    cfg: BotConfig,
    metrics: Arc<Metrics>,
    shutdown: Arc<Notify>,
) {
    // Auth.
    let auth_outcome = match auth::perform(&cfg.auth_addr, &username).await {
        Ok(o) => {
            metrics.auth_ok.fetch_add(1, Ordering::Relaxed);
            o
        }
        Err(e) => {
            metrics.auth_fail.fetch_add(1, Ordering::Relaxed);
            tracing::debug!("bot {slot} {username} auth failed: {e}");
            return;
        }
    };

    let world_addr = cfg
        .world_addr_override
        .clone()
        .unwrap_or(auth_outcome.realm_address.clone());

    // World handshake + char create + login.
    let session = match world::establish(&world_addr, &username, auth_outcome.session_key).await {
        Ok(s) => {
            metrics.world_ok.fetch_add(1, Ordering::Relaxed);
            s
        }
        Err(e) => {
            metrics.world_fail.fetch_add(1, Ordering::Relaxed);
            tracing::debug!("bot {slot} {username} world failed: {e}");
            return;
        }
    };

    metrics.bots_alive.fetch_add(1, Ordering::Relaxed);

    let world::WorldSession {
        reader,
        writer,
        encrypter,
        decrypter,
        character_guid: _,
    } = session;

    // Reader future: drain incoming encrypted messages until error/EOF.
    let read_metrics = metrics.clone();
    let read_fut = async move {
        let mut reader = reader;
        let mut decrypter = decrypter;
        loop {
            match ServerOpcodeMessage::tokio_read_encrypted(&mut reader, &mut decrypter).await {
                Ok(_) => {
                    read_metrics.messages_in.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::trace!("bot {slot} reader exit: {e:?}");
                    return;
                }
            }
        }
    };

    // Drive future: ticks movement + periodic ping.
    let drive_metrics = metrics.clone();
    let drive_fut = async move {
        let mut writer = writer;
        let mut encrypter = encrypter;
        let mut driver = MovementDriver::new(drive_metrics.clone());
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut ping = tokio::time::interval(Duration::from_secs(30));
        ping.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut ping_seq = 0u32;

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if let Err(e) = driver.tick(&mut writer, &mut encrypter).await {
                        tracing::trace!("bot {slot} drive error: {e}");
                        drive_metrics.send_errors.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                }
                _ = ping.tick() => {
                    ping_seq = ping_seq.wrapping_add(1);
                    let msg = CMSG_PING { sequence_id: ping_seq, round_time_in_ms: 0 };
                    if let Err(e) = msg.tokio_write_encrypted_client(&mut writer, &mut encrypter).await {
                        tracing::trace!("bot {slot} ping error: {e}");
                        drive_metrics.send_errors.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    drive_metrics.messages_out.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    };

    let shutdown_fut = shutdown.notified();

    tokio::pin!(read_fut);
    tokio::pin!(drive_fut);
    tokio::pin!(shutdown_fut);

    tokio::select! {
        _ = &mut read_fut => {}
        _ = &mut drive_fut => {}
        _ = &mut shutdown_fut => {}
    }

    metrics.bots_alive.fetch_sub(1, Ordering::Relaxed);
}
