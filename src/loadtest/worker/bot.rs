//! Single bot lifecycle: auth → world handshake → walk → shutdown.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use wow_world_messages::errors::ExpectedOpcodeError;
use wow_world_messages::vanilla::ClientMessage as _;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::{CMSG_COMPLETE_CINEMATIC, CMSG_PING};

use crate::worker::BotMode;
use crate::worker::Fleet;
use crate::worker::auth;
use crate::worker::metrics::Metrics;
use crate::worker::movement::{Mode, MovementDriver};
use crate::worker::pvp::PvpState;
use crate::worker::world;

pub mod race;

/// A write that hasn't finished within this means the server stopped
/// reading; the bot gives up rather than hanging.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct BotConfig {
    pub auth_addr: String,
    /// Override the realm address returned by the auth server. Useful when the
    /// auth server tells us `vpn.gtker.com:8085` but we actually want `127.0.0.1:8085`.
    pub world_addr_override: Option<String>,
    pub username_prefix: String,
    /// The accounts' password; `None` uses the username, as
    /// wow_vanilla_server's auto-created accounts and Thistle Tea's seeded
    /// `BOT0000`..`BOT0999` do.
    pub password: Option<String>,
    /// The whole login, from connecting to auth until the server confirms the
    /// character is in the world, must finish within this.
    pub login_timeout: Duration,
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
    /// Shared `VanillaMap` for Race mode: `Some` with maps configured;
    /// `None` on the hard-coded fallback path (no per-tick ground sampling).
    pub race_map: Option<Arc<rustigator::vanilla::VanillaMap>>,
}

pub struct BotHandle {
    /// Unique for the worker's lifetime, unlike the slab key or the slot,
    /// which are reused.
    pub id: u64,
    pub shutdown: Arc<Notify>,
    pub join: JoinHandle<()>,
}

/// What a bot gives back when it ends, however it ends: its slab entry, its
/// account slot, and its place in the alive count. A guard, so that a bot
/// aborted by `stop` or `drain` settles up too: aborting drops its future.
pub struct BotExit {
    pub fleet: Arc<Fleet>,
    pub key: usize,
    pub id: u64,
    pub slot: u32,
    pub metrics: Arc<Metrics>,
    pub alive: bool,
}

impl Drop for BotExit {
    fn drop(&mut self) {
        if self.alive {
            self.metrics.bots_alive.fetch_sub(1, Ordering::Relaxed);
        }
        self.fleet.release(self.key, self.id, self.slot);
    }
}

pub fn username_for(prefix: &str, slot: u32) -> String {
    format!("{}{:04}", prefix.to_uppercase(), slot)
}

/// Start a bot on `slot`. `exit` is handed back to the fleet when it ends.
pub fn spawn(slot: u32, cfg: BotConfig, metrics: Arc<Metrics>, exit: BotExit) -> BotHandle {
    let username = username_for(&cfg.username_prefix, slot);
    let shutdown = Arc::new(Notify::new());
    let id = exit.id;
    let join = {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            run_bot(slot, username, cfg, metrics, shutdown, exit).await;
        })
    };
    BotHandle { id, shutdown, join }
}

/// Where a login failed.
enum LoginError {
    Auth(std::io::Error),
    World(std::io::Error),
}

/// Something the reader saw that the driver must answer, since the driver
/// owns the socket's write half.
enum ReaderEvent {
    TriggerCinematic,
}

async fn run_bot(
    slot: u32,
    username: String,
    cfg: BotConfig,
    metrics: Arc<Metrics>,
    shutdown: Arc<Notify>,
    mut exit: BotExit,
) {
    let password = cfg.password.clone().unwrap_or_else(|| username.clone());
    let login = async {
        let auth_outcome = auth::perform(&cfg.auth_addr, &username, &password)
            .await
            .map_err(LoginError::Auth)?;
        metrics.auth_ok.fetch_add(1, Ordering::Relaxed);
        let world_addr = cfg
            .world_addr_override
            .clone()
            .unwrap_or(auth_outcome.realm_address.clone());
        world::establish(&world_addr, &username, slot, auth_outcome.session_key)
            .await
            .map_err(LoginError::World)
    };

    // Auth, world handshake, char create and login, bounded by the login
    // deadline and cancelled by `stop` or `drain`.
    let session = tokio::select! {
        r = tokio::time::timeout(cfg.login_timeout, login) => match r {
            Ok(Ok(s)) => s,
            Ok(Err(LoginError::Auth(e))) => {
                metrics.auth_fail.fetch_add(1, Ordering::Relaxed);
                tracing::debug!("bot {slot} {username} auth failed: {e}");
                return;
            }
            Ok(Err(LoginError::World(e))) => {
                metrics.world_fail.fetch_add(1, Ordering::Relaxed);
                tracing::debug!("bot {slot} {username} world failed: {e}");
                return;
            }
            Err(_) => {
                metrics.login_timeouts.fetch_add(1, Ordering::Relaxed);
                tracing::debug!("bot {slot} {username} login timed out");
                return;
            }
        },
        () = shutdown.notified() => return,
    };

    // The server has confirmed the character is in the world.
    metrics.world_ok.fetch_add(1, Ordering::Relaxed);
    metrics.skipped_in.fetch_add(session.skipped, Ordering::Relaxed);
    metrics.bots_alive.fetch_add(1, Ordering::Relaxed);
    exit.alive = true;

    let world::WorldSession {
        reader,
        writer,
        encrypter,
        decrypter,
        character_guid,
        map,
        position,
        orientation,
        cinematic,
        ..
    } = session;
    tracing::debug!(
        "bot {slot} {username} in the world: {map:?} ({:.1}, {:.1}, {:.1})",
        position.x,
        position.y,
        position.z
    );

    // Shared cache of (other-player guid → last-known position) populated
    // by the reader. Only allocated in PvP mode; `None` skips both the
    // parse-time bookkeeping AND the driver's pursuit branch.
    let pvp_state: Option<Arc<Mutex<PvpState>>> = if cfg.mode == BotMode::Pvp {
        Some(Arc::new(Mutex::new(PvpState::default())))
    } else {
        None
    };

    let (events_tx, mut events) = mpsc::unbounded_channel();
    if cinematic {
        let _ = events_tx.send(ReaderEvent::TriggerCinematic);
    }

    // Reader future: drain incoming encrypted messages. Only IO errors end
    // it. wow_messages reads a packet's whole body before dispatching it, so
    // a packet it doesn't model (`Opcode`) or can't parse (`Parse`) is
    // already consumed and is simply skipped, as a real client ignores what
    // it doesn't understand.
    let read_metrics = metrics.clone();
    let read_pvp = pvp_state.clone();
    let own_guid = character_guid;
    let read_fut = async move {
        let mut reader = reader;
        let mut decrypter = decrypter;
        loop {
            match ServerOpcodeMessage::tokio_read_encrypted(&mut reader, &mut decrypter).await {
                Ok(msg) => {
                    if matches!(msg, ServerOpcodeMessage::SMSG_TRIGGER_CINEMATIC(_)) {
                        let _ = events_tx.send(ReaderEvent::TriggerCinematic);
                    }
                    if movement_of(&msg).is_some_and(|(guid, _)| guid != own_guid) {
                        read_metrics.movement_in.fetch_add(1, Ordering::Relaxed);
                    }
                    if let Some(state) = read_pvp.as_ref() {
                        observe_movement(state, own_guid, &msg);
                    }
                    read_metrics.messages_in.fetch_add(1, Ordering::Relaxed);
                }
                Err(ExpectedOpcodeError::Opcode { opcode, size, name }) => {
                    read_metrics.skipped_in.fetch_add(1, Ordering::Relaxed);
                    tracing::trace!(
                        "bot {slot} skipped unhandled opcode {name:?} (0x{opcode:X}, {size} bytes)"
                    );
                }
                Err(ExpectedOpcodeError::Parse(e)) => {
                    read_metrics.parse_errors.fetch_add(1, Ordering::Relaxed);
                    tracing::trace!("bot {slot} skipped unparseable packet: {e:?}");
                }
                Err(ExpectedOpcodeError::Io(e)) => {
                    tracing::trace!("bot {slot} reader exit (io): {e}");
                    return;
                }
            }
        }
    };

    // Drive future: ticks movement + periodic ping, and answers what the
    // reader saw.
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
            },
            BotMode::Random => Mode::Random,
        };
        let mut driver = MovementDriver::new(drive_metrics.clone(), mode, position, orientation);
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut ping = tokio::time::interval(Duration::from_secs(30));
        ping.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut ping_seq = 0u32;

        loop {
            let sent = tokio::select! {
                _ = tick.tick() => {
                    tokio::time::timeout(WRITE_TIMEOUT, driver.tick(&mut writer, &mut encrypter)).await
                }
                _ = ping.tick() => {
                    ping_seq = ping_seq.wrapping_add(1);
                    let msg = CMSG_PING { sequence_id: ping_seq, round_time_in_ms: 0 };
                    drive_metrics.messages_out.fetch_add(1, Ordering::Relaxed);
                    tokio::time::timeout(
                        WRITE_TIMEOUT,
                        msg.tokio_write_encrypted_client(&mut writer, &mut encrypter),
                    )
                    .await
                }
                Some(event) = events.recv() => match event {
                    // A bot has no use for the intro: end it, as a client
                    // does when it finishes. Some servers keep the
                    // player's view on the camera until then.
                    ReaderEvent::TriggerCinematic => {
                        drive_metrics.messages_out.fetch_add(1, Ordering::Relaxed);
                        tokio::time::timeout(
                            WRITE_TIMEOUT,
                            CMSG_COMPLETE_CINEMATIC {}.tokio_write_encrypted_client(&mut writer, &mut encrypter),
                        )
                        .await
                    }
                },
            };
            match sent {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::trace!("bot {slot} write error: {e}");
                    drive_metrics.send_errors.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(_) => {
                    tracing::trace!("bot {slot} write timed out");
                    drive_metrics.send_errors.fetch_add(1, Ordering::Relaxed);
                    return;
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
    // `exit` settles up as it drops.
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

    let Some((guid, pos)) = movement_of(msg) else {
        return;
    };
    if guid == own_guid {
        return;
    }
    if let Ok(mut state) = state.lock() {
        state.observe(guid, pos);
    }
}

/// The mover and its position, if `msg` is a movement broadcast.
fn movement_of(
    msg: &ServerOpcodeMessage,
) -> Option<(wow_world_messages::Guid, wow_world_messages::vanilla::Vector3d)> {
    Some(match msg {
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
        _ => return None,
    })
}
