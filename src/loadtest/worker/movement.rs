//! Random-walk movement driver for a single bot.
//!
//! Sits inside the bot's drive task. Picks a new walking direction every
//! few seconds, emits `MSG_MOVE_START_*` / `MSG_MOVE_STOP_Client` on
//! transitions, and a heartbeat every 250 ms while moving.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::RngExt;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use wow_srp::vanilla_header::EncrypterHalf;
use wow_world_messages::vanilla::ClientMessage as _;
use wow_world_messages::vanilla::{
    MSG_MOVE_HEARTBEAT_Client, MSG_MOVE_START_FORWARD_Client, MSG_MOVE_START_STRAFE_LEFT_Client,
    MSG_MOVE_START_STRAFE_RIGHT_Client, MSG_MOVE_STOP_Client, MovementInfo,
    MovementInfo_MovementFlags, Vector3d,
};

use crate::worker::metrics::Metrics;

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

#[derive(Debug, Clone, Copy)]
enum Phase {
    Idle,
    Forward,
    StrafeLeft,
    StrafeRight,
}

pub struct MovementDriver {
    info: MovementInfo,
    phase: Phase,
    phase_ends_at: Instant,
    last_heartbeat_at: Instant,
    started_at: Instant,
    metrics: Arc<Metrics>,
}

impl MovementDriver {
    pub fn new(metrics: Arc<Metrics>) -> Self {
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
