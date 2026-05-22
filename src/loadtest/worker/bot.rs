//! Single bot lifecycle: auth → world handshake → walk → shutdown.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use wow_world_messages::errors::ExpectedOpcodeError;
use wow_world_messages::vanilla::ClientMessage as _;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::CMSG_PING;

use crate::worker::BotMode;
use crate::worker::auth;
use crate::worker::metrics::Metrics;
use crate::worker::movement::{Mode, MovementDriver};
use crate::worker::pvp::PvpState;
use crate::worker::world;

pub mod race;

#[derive(Debug, Clone)]
pub struct BotConfig {
    pub auth_addr: String,
    /// Override the realm address returned by the auth server. Useful when the
    /// auth server tells us `vpn.gtker.com:8085` but we actually want `127.0.0.1:8085`.
    pub world_addr_override: Option<String>,
    pub username_prefix: String,
    /// Which behavior the bot runs — random walk, PvP, or the BB→SW race.
    pub mode: BotMode,
    /// Worker-wide "battle start" latch. Bots in PvP mode block in their
    /// gather phase until the worker sets this to `true` (typically after
    /// the initial spawn batch finishes). Unused outside PvP mode but
    /// always present so we can drop `Option<Arc<_>>` plumbing — the cost
    /// of an unused Arc clone is trivial.
    pub battle_started: Arc<AtomicBool>,
    /// Shared waypoint list for Race mode. Empty for other modes.
    pub race_path: Arc<[wow_world_messages::vanilla::Vector3d]>,
    /// Shared `VanillaMap` handle for Race mode. `Some` when
    /// namigator wired up cleanly; `None` when running the
    /// hardcoded-fallback path (no per-tick ground sampling).
    pub race_map: Option<Arc<std::sync::Mutex<rustigator::vanilla::VanillaMap>>>,
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
        character_guid,
    } = session;

    // Shared cache of (other-player guid → last-known position) populated
    // by the reader. Only allocated in PvP mode; `None` skips both the
    // parse-time bookkeeping AND the driver's pursuit branch.
    let pvp_state: Option<Arc<Mutex<PvpState>>> = if cfg.mode == BotMode::Pvp {
        Some(Arc::new(Mutex::new(PvpState::default())))
    } else {
        None
    };

    // Reader future: drain incoming encrypted messages. Only IO errors
    // exit the loop — parse errors and unknown opcodes get skipped, the
    // same way a real WoW client tolerates server packets it doesn't
    // understand. Without this the bot's read task previously exited on
    // ANY error (including a single unmodeled opcode from the server),
    // which dropped the TCP connection and made the server log them as
    // "stale clients". That capped sustained bot counts around ~510 in
    // an 800-bot ramp.
    let read_metrics = metrics.clone();
    let read_pvp = pvp_state.clone();
    let own_guid = character_guid;
    let read_fut = async move {
        let mut reader = reader;
        let mut decrypter = decrypter;
        loop {
            match ServerOpcodeMessage::tokio_read_encrypted(&mut reader, &mut decrypter).await {
                Ok(msg) => {
                    if let Some(state) = read_pvp.as_ref() {
                        observe_movement(state, own_guid, &msg);
                    }
                    read_metrics.messages_in.fetch_add(1, Ordering::Relaxed);
                }
                Err(ExpectedOpcodeError::Opcode { opcode, size, name }) => {
                    // Skip the body so the stream cursor lines up with the
                    // next opcode boundary. If the body read fails we
                    // disconnect for real.
                    let mut body = vec![0_u8; size as usize];
                    if reader.read_exact(&mut body).await.is_err() {
                        return;
                    }
                    tracing::trace!(
                        "bot {slot} skipped unhandled opcode {name:?} (0x{opcode:X}, {size} bytes)"
                    );
                }
                Err(ExpectedOpcodeError::Parse(e)) => {
                    tracing::trace!("bot {slot} parse error: {e:?}");
                    // Parse errors usually mean we lost frame sync — bail
                    // out for the bot rather than trying to resynchronize.
                    return;
                }
                Err(ExpectedOpcodeError::Io(e)) => {
                    tracing::trace!("bot {slot} reader exit (io): {e}");
                    return;
                }
            }
        }
    };

    // Drive future: ticks movement + periodic ping.
    let drive_metrics = metrics.clone();
    let drive_pvp = pvp_state.clone();
    let drive_fut = async move {
        let mut writer = writer;
        let mut encrypter = encrypter;
        let mode = match cfg.mode {
            BotMode::Pvp => Mode::Pvp {
                state: drive_pvp.expect("PvP mode allocates pvp_state above"),
                own_guid: character_guid,
                battle_started: cfg.battle_started.clone(),
            },
            BotMode::Race => Mode::Race {
                path: cfg.race_path.clone(),
                index: 0,
                forward: true,
                jitter: race::jitter_for_slot(slot),
                teleported: false,
                map: cfg.race_map.clone(),
                hb_diag_count: 0,
                bot_slot: slot,
            },
            BotMode::Random => Mode::Random,
        };
        let mut driver = MovementDriver::new(drive_metrics.clone(), mode);
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

/// Pluck (guid, position) pairs out of incoming movement opcodes and feed
/// them into the bot's shared `PvpState`. Skips the bot's own guid (we
/// don't want to chase ourselves). Also accumulates inbound damage
/// targeted at us so the driver can detect its own death and trigger a
/// ring-respawn. Only called in PvP mode — keeping the parse out of the
/// no-PvP hot path is a deliberate ~zero-cost gate.
fn observe_movement(state: &Mutex<PvpState>, own_guid: wow_world_messages::Guid, msg: &ServerOpcodeMessage) {
    // Combat-log accounting first — separate from movement. Routes both
    // self-damage (drives our own death state) and damage-to-target
    // (drops the target lock when the lock-target dies).
    if let ServerOpcodeMessage::SMSG_ATTACKERSTATEUPDATE(s) = msg
        && let Ok(mut state) = state.lock()
    {
        state.record_attack_seen(s.attacker, s.target, s.total_damage, own_guid);
    }

    let (guid, pos) = match msg {
        ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_START_FORWARD(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_START_BACKWARD(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_STOP(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_START_STRAFE_LEFT(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_START_STRAFE_RIGHT(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_STOP_STRAFE(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_JUMP(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_START_TURN_LEFT(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_START_TURN_RIGHT(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_STOP_TURN(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_START_PITCH_UP(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_START_PITCH_DOWN(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_STOP_PITCH(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_SET_RUN_MODE(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_SET_WALK_MODE(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_SET_FACING(m) => (m.guid, m.info.position),
        ServerOpcodeMessage::MSG_MOVE_SET_PITCH(m) => (m.guid, m.info.position),
        _ => return,
    };
    if guid == own_guid {
        return;
    }
    if let Ok(mut state) = state.lock() {
        state.observe(guid, pos);
    }
}
