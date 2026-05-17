//! Random-walk or PvP movement driver for a single bot.
//!
//! Sits inside the bot's drive task. Two modes:
//!
//! - **`Mode::Random`** — pick a new walking direction every few seconds,
//!   emit `MSG_MOVE_START_*` / `MSG_MOVE_STOP_Client` on transitions, send
//!   a heartbeat every 250 ms while moving. Drifts inside a 60yd box
//!   around the spawn anchor.
//! - **`Mode::Pvp`** — peek at the shared [`PvpState`] cache for nearby
//!   players, lock onto a random one, run toward their last-known position,
//!   and send `CMSG_ATTACKSWING` once within melee range. Falls back to
//!   random-walk while no targets are visible so bots actually move and
//!   become discoverable to each other.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::RngExt;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use wow_srp::vanilla_header::EncrypterHalf;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::ClientMessage as _;
use wow_world_messages::vanilla::{
    CMSG_ATTACKSWING, MSG_MOVE_HEARTBEAT_Client, MSG_MOVE_START_FORWARD_Client,
    MSG_MOVE_START_STRAFE_LEFT_Client, MSG_MOVE_START_STRAFE_RIGHT_Client, MSG_MOVE_STOP_Client,
    MovementInfo, MovementInfo_MovementFlags, Vector3d,
};

use crate::worker::metrics::Metrics;
use crate::worker::pvp::PvpState;

/// Anchor for the random-walk: Gurubashi Arena spawn point. Must match the
/// server-side override in
/// `src/world/character_screen_handler/char_create.rs::SPAWN_POSITION` so
/// the bot's first heartbeat doesn't immediately position-correct the
/// player away from where the server placed them at character creation.
const ANCHOR: Vector3d = Vector3d {
    x: -13206.0,
    y: 272.0,
    z: 21.857,
};

/// Maximum distance from the anchor before we snap back. 60 yd matches the
/// plan's "60 yd box" — large enough that bots feel like they're roaming,
/// small enough that they cluster within AOI of each other.
const MAX_DRIFT_YARDS: f32 = 60.0;

/// Player running speed in yd/s. **Must match what observer clients use to
/// extrapolate the bot's position between heartbeats** — otherwise every
/// heartbeat corrects the visible position backward, producing systemic
/// 4 Hz rubber-banding. We send `new_forward()` (run, not walk), so observer
/// clients interpolate at `DEFAULT_RUNNING_SPEED` from the player's update
/// mask, and the local advance has to match.
const RUN_SPEED: f32 = wow_world_base::movement::DEFAULT_RUNNING_SPEED;

const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(250);

/// Distance at which a PvP bot stops pursuing and starts swinging. The
/// server doesn't currently range-check melee swings, so this is purely
/// the bot's own behavior — picking a value close to vanilla melee reach
/// keeps the visual sensible without making fights chase forever.
const PVP_ATTACK_RANGE: f32 = 5.0;

/// How often a bot in PvP mode reconsiders its target. Re-evaluating
/// every tick would flap targets every time someone closer drifted into
/// view; once every few seconds is plenty of action.
const PVP_TARGET_REFRESH: Duration = Duration::from_secs(4);

/// Minimum gap between consecutive `CMSG_ATTACKSWING` packets from the
/// bot. The server caps swing damage by `UNARMED_SPEED` already, but
/// sending faster than that just wastes bandwidth.
const PVP_SWING_INTERVAL: Duration =
    Duration::from_millis((wow_world_base::combat::UNARMED_SPEED * 1000.0) as u64);

/// Match the server's `RESPAWN_DELAY` constant in
/// `src/world/world_opcode_handler/character.rs`. The loadtest binary
/// can't import from the server's module tree, so this is a hand-synced
/// copy. We add 500 ms of pad before the ring-teleport actually fires,
/// to make sure the server has already gone through its alive-restore
/// pass.
const RESPAWN_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
enum Phase {
    Idle,
    Forward,
    StrafeLeft,
    StrafeRight,
}

/// Behavioral mode the driver runs in. Picked at bot start and never
/// changes for the bot's lifetime — toggling modes mid-run would need
/// orchestrator support that nobody's asked for yet.
pub enum Mode {
    /// Existing random-walk inside a 60yd box around the spawn anchor.
    Random,
    /// Pursue + attack other players. `state` is the shared
    /// observer-position cache populated by the bot's read task;
    /// `own_guid` is excluded from target selection. `battle_started`
    /// is a worker-wide latch — while false, bots gather in the pit
    /// (strafe to a random point within 30yd of the arena center) and
    /// wait for the worker to flip the latch after the initial spawn
    /// batch finishes.
    Pvp {
        state: Arc<Mutex<PvpState>>,
        own_guid: Guid,
        battle_started: Arc<AtomicBool>,
    },
}

/// Radius inside the arena pit that bots strafe into during the
/// pre-battle gather phase. The pit floor is ~25 yd radius, so 30 yd
/// just kisses the edge — bots are still in the pit but distributed.
const PVP_GATHER_RADIUS: f32 = 30.0;

/// Arrival threshold for the gather strafe. Closer than this we stop
/// and stand still until the battle latch flips.
const PVP_GATHER_ARRIVED_DIST_SQ: f32 = 1.0;

pub struct MovementDriver {
    info: MovementInfo,
    phase: Phase,
    phase_ends_at: Instant,
    last_heartbeat_at: Instant,
    started_at: Instant,
    metrics: Arc<Metrics>,
    mode: Mode,
    /// PvP only: the guid the bot is currently chasing. `None` falls
    /// back to random-walk so the bot moves around and discovers others.
    current_target: Option<Guid>,
    /// PvP only: monotonic clock at which we'll consider refreshing
    /// `current_target`.
    target_refresh_at: Instant,
    /// PvP only: monotonic clock at which we'll consider issuing the
    /// next swing — guards against firing faster than `UNARMED_SPEED`.
    next_swing_at: Instant,
    /// PvP only, pre-battle: random point inside the 30yd gather radius
    /// the bot strafes toward during the gather phase. `None` until the
    /// first gather tick rolls it.
    gather_destination: Option<Vector3d>,
}

impl MovementDriver {
    pub fn new(metrics: Arc<Metrics>, mode: Mode) -> Self {
        let orientation = rand::rng().random_range(0.0_f32..std::f32::consts::TAU);
        let now = Instant::now();
        Self {
            info: MovementInfo {
                flags: MovementInfo_MovementFlags::empty(),
                timestamp: 0,
                position: ANCHOR,
                orientation,
                fall_time: 0.0,
            },
            phase: Phase::Idle,
            phase_ends_at: now,
            last_heartbeat_at: now,
            started_at: now,
            metrics,
            mode,
            current_target: None,
            target_refresh_at: now,
            next_swing_at: now,
            gather_destination: None,
        }
    }

    pub async fn tick(
        &mut self,
        writer: &mut OwnedWriteHalf,
        encrypter: &mut EncrypterHalf,
    ) -> std::io::Result<()> {
        let now = Instant::now();
        self.info.timestamp =
            u32::try_from(now.duration_since(self.started_at).as_millis() & 0xFFFF_FFFF).unwrap_or(0);

        match &self.mode {
            Mode::Random => self.tick_random(writer, encrypter, now).await,
            Mode::Pvp { .. } => self.tick_pvp(writer, encrypter, now).await,
        }
    }

    async fn tick_random(
        &mut self,
        writer: &mut OwnedWriteHalf,
        encrypter: &mut EncrypterHalf,
        now: Instant,
    ) -> std::io::Result<()> {
        if now >= self.phase_ends_at {
            self.transition_phase(writer, encrypter, now).await?;
        }

        let is_moving = !matches!(self.phase, Phase::Idle);
        if is_moving && now.duration_since(self.last_heartbeat_at) >= HEARTBEAT_INTERVAL {
            self.advance_position(now);
            let msg = MSG_MOVE_HEARTBEAT_Client {
                info: self.info.clone(),
            };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                .await?;
            writer.flush().await?;
            self.metrics
                .messages_out
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.last_heartbeat_at = now;
        }

        Ok(())
    }

    /// PvP tick: detect death, schedule a ring-respawn, otherwise pursue a
    /// random target from the shared `PvpState` and swing once in melee
    /// range. While no target is visible we fall back to the random-walk
    /// driver so the bot keeps moving and discovers others through their
    /// position broadcasts.
    async fn tick_pvp(
        &mut self,
        writer: &mut OwnedWriteHalf,
        encrypter: &mut EncrypterHalf,
        now: Instant,
    ) -> std::io::Result<()> {
        // Gather phase: worker hasn't flipped `battle_started` yet, so
        // every bot scatters into the pit and stands still until the
        // signal. Coordinated start ensures both sides of every fight
        // are alive when combat begins — no kills against bots that
        // haven't finished spawning.
        let battle_on = match &self.mode {
            Mode::Pvp { battle_started, .. } => battle_started.load(Ordering::Relaxed),
            _ => unreachable!("tick_pvp called outside Mode::Pvp"),
        };
        if !battle_on {
            return self.tick_gather(writer, encrypter, now).await;
        }

        // Snapshot the state we need under the lock — no awaits while
        // holding it. Stuff we read: are we dead, when did we die, what
        // does our current target's last-known position look like.
        let (own_guid, is_dead, died_at, target_pos) = {
            let (state, own_guid) = match &self.mode {
                Mode::Pvp { state, own_guid, .. } => (state, *own_guid),
                _ => unreachable!("tick_pvp called outside Mode::Pvp"),
            };
            let state = state.lock().expect("pvp state mutex poisoned");
            let is_dead = state.last_death_at.is_some();
            let died_at = state.last_death_at;
            let target_pos = self
                .current_target
                .and_then(|g| state.position_of(g));
            (own_guid, is_dead, died_at, target_pos)
        };

        // Branch 1: respawn flow. We've been dead for at least
        // `RESPAWN_DELAY` server-side; pick a ring position around the
        // arena, teleport the bot client-side, clear death flags.
        if is_dead {
            if let Some(t) = died_at {
                // Pad RESPAWN_DELAY by 500 ms so the server has definitely
                // gone through its own resurrect pass before we start
                // sending movement again.
                if now.duration_since(t) >= RESPAWN_DELAY + Duration::from_millis(500) {
                    self.respawn_to_ring(writer, encrypter, own_guid, now)
                        .await?;
                }
            }
            return Ok(());
        }

        // Branch 2: alive. Refresh target every PVP_TARGET_REFRESH or when
        // we have none, drop stale ones (the lock takes care of that).
        if self.current_target.is_none() || now >= self.target_refresh_at {
            let new_target = match &self.mode {
                Mode::Pvp { state, own_guid, .. } => {
                    let mut state = state.lock().expect("pvp state mutex poisoned");
                    state.pick_random_target(*own_guid)
                }
                _ => unreachable!(),
            };
            self.current_target = new_target.map(|(g, _)| g);
            self.target_refresh_at = now + PVP_TARGET_REFRESH;
        }

        // Branch 2a: no target visible yet — fall back to random-walk so
        // we generate position broadcasts other bots can latch onto.
        let Some(target_pos) = target_pos else {
            return self.tick_random(writer, encrypter, now).await;
        };

        // Branch 2b: pursue. Aim at target, run forward, swing when close.
        let dx = target_pos.x - self.info.position.x;
        let dy = target_pos.y - self.info.position.y;
        let dist_sq = dx * dx + dy * dy;
        let new_orientation = dy.atan2(dx);

        // Re-issue START_FORWARD if either we weren't moving or the
        // heading drifted noticeably. The server reapplies orientation
        // from every move opcode, so we don't need to spam SET_FACING.
        let needs_reorient = !matches!(self.phase, Phase::Forward)
            || (self.info.orientation - new_orientation).abs() > 0.15;
        if needs_reorient {
            self.info.orientation = new_orientation;
            self.info.flags = MovementInfo_MovementFlags::new_forward();
            self.phase = Phase::Forward;
            let msg = MSG_MOVE_START_FORWARD_Client {
                info: self.info.clone(),
            };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                .await?;
            self.metrics
                .messages_out
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        // Advance our local position + emit heartbeat at the same cadence
        // as random-walk so observers can interpolate smoothly.
        if now.duration_since(self.last_heartbeat_at) >= HEARTBEAT_INTERVAL {
            self.advance_position(now);
            let msg = MSG_MOVE_HEARTBEAT_Client {
                info: self.info.clone(),
            };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                .await?;
            self.metrics
                .messages_out
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.last_heartbeat_at = now;
        }

        // In melee range? Fire a swing — but never faster than the server
        // would accept it, hence the per-swing rate cap. Self-targets are
        // dropped server-side so guarding here isn't strictly needed.
        if dist_sq <= PVP_ATTACK_RANGE * PVP_ATTACK_RANGE
            && now >= self.next_swing_at
            && let Some(target_guid) = self.current_target
        {
            let msg = CMSG_ATTACKSWING { guid: target_guid };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                .await?;
            self.metrics
                .messages_out
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.next_swing_at = now + PVP_SWING_INTERVAL;
        }

        writer.flush().await?;
        Ok(())
    }

    /// Pre-battle gather phase. Each bot picks a random point in a 30yd
    /// circle around the arena center (uniform across the disc) and
    /// strafes to it. Strafing rather than running so the bots look like
    /// they're shuffling into position — minor cosmetic, but the user
    /// asked for it. Bots stand still once arrived; the gather phase
    /// ends when the worker flips `battle_started`.
    async fn tick_gather(
        &mut self,
        writer: &mut OwnedWriteHalf,
        encrypter: &mut EncrypterHalf,
        now: Instant,
    ) -> std::io::Result<()> {
        if self.gather_destination.is_none() {
            // Uniform-over-disc: `r = R * sqrt(u)` instead of `r = R * u`,
            // so density doesn't bunch toward the center.
            let (angle, radius_norm) = {
                let mut rng = rand::rng();
                let a = rng.random_range(0.0_f32..std::f32::consts::TAU);
                let u: f32 = rng.random_range(0.0_f32..1.0_f32);
                (a, u.sqrt())
            };
            let r = radius_norm * PVP_GATHER_RADIUS;
            self.gather_destination = Some(Vector3d {
                x: ANCHOR.x + r * angle.cos(),
                y: ANCHOR.y + r * angle.sin(),
                z: ANCHOR.z,
            });
        }
        let dest = self
            .gather_destination
            .expect("gather destination set above");

        let dx = dest.x - self.info.position.x;
        let dy = dest.y - self.info.position.y;
        let dist_sq = dx * dx + dy * dy;

        // Arrived: stop and stand still until the battle latch flips.
        if dist_sq <= PVP_GATHER_ARRIVED_DIST_SQ {
            if !matches!(self.phase, Phase::Idle) {
                self.info.flags = MovementInfo_MovementFlags::empty();
                self.phase = Phase::Idle;
                let msg = MSG_MOVE_STOP_Client {
                    info: self.info.clone(),
                };
                msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                    .await?;
                writer.flush().await?;
                self.metrics
                    .messages_out
                    .fetch_add(1, Ordering::Relaxed);
            }
            return Ok(());
        }

        // Strafe-left toward the destination. With orientation set to
        // `dir_to_dest - PI/2`, `advance_position`'s strafe-left vector
        // `(-sin(o), cos(o))` evaluates to `(cos(dir), sin(dir))` — the
        // direction-to-destination unit vector. So the strafe-left phase
        // moves us toward `dest` while the bot faces 90° to the side.
        let dir_angle = dy.atan2(dx);
        let new_orientation = dir_angle - std::f32::consts::FRAC_PI_2;
        let needs_transition = !matches!(self.phase, Phase::StrafeLeft)
            || (self.info.orientation - new_orientation).abs() > 0.15;
        if needs_transition {
            self.info.orientation = new_orientation;
            self.info.flags = MovementInfo_MovementFlags::new_strafe_left();
            self.phase = Phase::StrafeLeft;
            let msg = MSG_MOVE_START_STRAFE_LEFT_Client {
                info: self.info.clone(),
            };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                .await?;
            self.metrics
                .messages_out
                .fetch_add(1, Ordering::Relaxed);
        }

        if now.duration_since(self.last_heartbeat_at) >= HEARTBEAT_INTERVAL {
            self.advance_position(now);
            let msg = MSG_MOVE_HEARTBEAT_Client {
                info: self.info.clone(),
            };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                .await?;
            self.metrics
                .messages_out
                .fetch_add(1, Ordering::Relaxed);
            self.last_heartbeat_at = now;
        }

        writer.flush().await?;
        Ok(())
    }

    /// Teleport the bot to a uniformly-random ring position around the
    /// Gurubashi Arena (radius 70-110yd from `ANCHOR`, z fixed at the
    /// spectator-ring height of ~120). We "teleport" by overwriting our
    /// local `info.position` and immediately emitting a
    /// `MSG_MOVE_START_FORWARD` — the server trusts client-asserted
    /// positions, so this snaps the bot to the new location for every
    /// observer on the next tick. Clears death state when done.
    async fn respawn_to_ring(
        &mut self,
        writer: &mut OwnedWriteHalf,
        encrypter: &mut EncrypterHalf,
        _own_guid: Guid,
        now: Instant,
    ) -> std::io::Result<()> {
        // Roll the random ring position up-front and drop the rng before
        // the first `.await` — `ThreadRng` is `!Send` and would
        // otherwise infect the future across the wire write.
        let (angle, radius) = {
            let mut rng = rand::rng();
            let angle = rng.random_range(0.0_f32..std::f32::consts::TAU);
            // Inclusive-exclusive — 110 is the outer edge of the spectator
            // bowl rim before the slope falls away to ground level.
            let radius = rng.random_range(70.0_f32..110.0_f32);
            (angle, radius)
        };
        let new_x = ANCHOR.x + radius * angle.cos();
        let new_y = ANCHOR.y + radius * angle.sin();
        // Face the arena center so respawned bots line the ring as
        // spectators looking inward. Vector from new position to ANCHOR
        // is `-(cos(angle), sin(angle))`, whose direction is `angle + PI`.
        let new_orientation = (angle + std::f32::consts::PI) % std::f32::consts::TAU;
        // The arena's WMO spectator ring sits at z ≈ 120. Server-side
        // WMO clipping would let us land exactly on the rim mesh, but
        // the bot doesn't have namigator linked — so we use the flat
        // value the user picked as the documented fallback.
        const SPECTATOR_RING_Z: f32 = 120.0;

        self.info.position = Vector3d {
            x: new_x,
            y: new_y,
            z: SPECTATOR_RING_Z,
        };
        self.info.orientation = new_orientation;
        self.info.flags = MovementInfo_MovementFlags::new_forward();
        self.phase = Phase::Forward;
        self.last_heartbeat_at = now;
        self.next_swing_at = now;
        self.current_target = None;
        self.target_refresh_at = now;

        let msg = MSG_MOVE_START_FORWARD_Client {
            info: self.info.clone(),
        };
        msg.tokio_write_encrypted_client(&mut *writer, encrypter)
            .await?;
        writer.flush().await?;
        self.metrics
            .messages_out
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Clear death bookkeeping AFTER the wire write so any unexpected
        // io error keeps us "dead" for the next tick to retry.
        if let Mode::Pvp { state, .. } = &self.mode
            && let Ok(mut state) = state.lock()
        {
            state.mark_respawned();
        }
        Ok(())
    }

    /// Decide a new phase and emit the corresponding START/STOP opcode.
    async fn transition_phase(
        &mut self,
        writer: &mut OwnedWriteHalf,
        encrypter: &mut EncrypterHalf,
        now: Instant,
    ) -> std::io::Result<()> {
        // Compute all random choices up-front so the `ThreadRng` (which is
        // `!Send`) is dropped before any `.await` point.
        let (next_phase, orientation, duration_ms) = {
            let mut rng = rand::rng();
            let next_phase = if rng.random_bool(0.2) {
                Phase::Idle
            } else {
                match rng.random_range(0..3) {
                    0 => Phase::Forward,
                    1 => Phase::StrafeLeft,
                    _ => Phase::StrafeRight,
                }
            };
            let orientation = rng.random_range(0.0_f32..std::f32::consts::TAU);
            let duration_ms = match next_phase {
                Phase::Idle => rng.random_range(500..2000),
                _ => rng.random_range(1000..4000),
            };
            (next_phase, orientation, duration_ms)
        };

        // From any moving phase, first issue a STOP so the server resets its
        // movement model.
        if !matches!(self.phase, Phase::Idle) {
            self.info.flags = MovementInfo_MovementFlags::empty();
            let msg = MSG_MOVE_STOP_Client {
                info: self.info.clone(),
            };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                .await?;
            self.metrics
                .messages_out
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        self.phase = next_phase;
        self.info.orientation = orientation;
        self.phase_ends_at = now + Duration::from_millis(duration_ms);

        match next_phase {
            Phase::Idle => {
                self.info.flags = MovementInfo_MovementFlags::empty();
            }
            Phase::Forward => {
                self.info.flags = MovementInfo_MovementFlags::new_forward();
                let msg = MSG_MOVE_START_FORWARD_Client {
                    info: self.info.clone(),
                };
                msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                    .await?;
                self.metrics
                    .messages_out
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Phase::StrafeLeft => {
                self.info.flags = MovementInfo_MovementFlags::new_strafe_left();
                let msg = MSG_MOVE_START_STRAFE_LEFT_Client {
                    info: self.info.clone(),
                };
                msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                    .await?;
                self.metrics
                    .messages_out
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Phase::StrafeRight => {
                self.info.flags = MovementInfo_MovementFlags::new_strafe_right();
                let msg = MSG_MOVE_START_STRAFE_RIGHT_Client {
                    info: self.info.clone(),
                };
                msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                    .await?;
                self.metrics
                    .messages_out
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        writer.flush().await?;
        self.last_heartbeat_at = now;
        Ok(())
    }

    /// Move the puppet along its current orientation, clamped to the anchor box.
    fn advance_position(&mut self, now: Instant) {
        let dt = now
            .duration_since(self.last_heartbeat_at)
            .as_secs_f32()
            .min(1.0);
        let (sin, cos) = self.info.orientation.sin_cos();
        let step = RUN_SPEED * dt;
        let (dx, dy) = match self.phase {
            Phase::Forward => (cos * step, sin * step),
            Phase::StrafeLeft => (-sin * step, cos * step),
            Phase::StrafeRight => (sin * step, -cos * step),
            Phase::Idle => (0.0, 0.0),
        };
        self.info.position.x += dx;
        self.info.position.y += dy;

        let drift = ((self.info.position.x - ANCHOR.x).powi(2)
            + (self.info.position.y - ANCHOR.y).powi(2))
        .sqrt();
        if drift > MAX_DRIFT_YARDS {
            // Bounce: reset to anchor and pick a new heading; cheaper than
            // reflecting against a virtual wall.
            self.info.position = ANCHOR;
        }
    }
}
