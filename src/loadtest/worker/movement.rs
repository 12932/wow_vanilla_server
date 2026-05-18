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

/// Race mode: when the bot is within this distance of the current
/// waypoint, advance to the next one. 5 yd is comfortably wider than
/// one heartbeat's worth of `RUN_SPEED × HEARTBEAT_INTERVAL` ≈ 1.75 yd
/// — so a bot never overshoots a waypoint between heartbeats and ends
/// up oscillating.
const RACE_WAYPOINT_RADIUS: f32 = 5.0;

/// Cached-distance threshold at which a PvP bot stops pursuing and
/// starts swinging. Matches the server's both-parties-moving melee
/// range (`ATTACK_DISTANCE 5.0 + MELEE_LEEWAY 8/3 ≈ 7.67 yd`, rounded
/// down to 7.5 for headroom). Stopping at 5 yd was too strict — by the
/// time the bot's cached position said 5 yd, real positions were
/// drifting around 6-8 yd and swings never landed. Stopping at 7.5
/// gives the server the chance to apply the leeway and resolve a hit.
const PVP_ATTACK_RANGE: f32 = 7.5;

/// Hard outer boundary for PvP bots. Position advance in `advance_position`
/// is clamped so a bot can never cross this radius from `ANCHOR` — chasing
/// an escapee won't carry the chaser out of the arena, which previously
/// produced runaway pursuit trains that left the pit and never returned.
/// Slightly smaller than the pit's ~25yd rim so we stay well inside the
/// playable zone.
const PVP_ARENA_RADIUS: f32 = 20.0;

/// Minimum gap between consecutive `CMSG_ATTACKSWING` packets from the
/// bot. The server caps swing damage by `UNARMED_SPEED` already, but
/// sending faster than that just wastes bandwidth.
const PVP_SWING_INTERVAL: Duration =
    Duration::from_millis((wow_world_base::combat::UNARMED_SPEED * 1000.0) as u64);

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
    /// (strafe to a random point within `PVP_GATHER_RADIUS` of the arena
    /// center) and wait for the worker to flip the latch after the
    /// initial spawn batch finishes.
    Pvp {
        state: Arc<Mutex<PvpState>>,
        own_guid: Guid,
        battle_started: Arc<AtomicBool>,
    },
    /// Amazing Race: walk a shared waypoint polyline from Booty Bay to
    /// Stormwind and back. The bot self-teleports to `path[0] + jitter`
    /// on its first tick (single heartbeat — server accepts it as a
    /// giant client-side teleport) and then runs forward along the
    /// path at `DEFAULT_RUNNING_SPEED`, switching waypoints when within
    /// `RACE_WAYPOINT_RADIUS`. On reaching the final waypoint the bot
    /// flips `forward` and runs the reverse direction.
    Race {
        path: Arc<[Vector3d]>,
        index: usize,
        forward: bool,
        jitter: (f32, f32),
        /// False until the first tick fires the initial teleport
        /// heartbeat. After that we track position client-side and
        /// only emit heartbeats on the normal `HEARTBEAT_INTERVAL`.
        teleported: bool,
    },
}

/// Radius inside the arena pit that bots strafe into during the
/// pre-battle gather phase. ~30 yd spreads bots across the full pit
/// rather than crammed near center; combined with retaliation
/// targeting, bots reliably engage even when initial separation is
/// 20+ yd.
const PVP_GATHER_RADIUS: f32 = 30.0;

/// Arrival threshold for the gather strafe. Closer than this we stop
/// and stand still until the battle latch flips.
const PVP_GATHER_ARRIVED_DIST_SQ: f32 = 1.0;

/// Maximum time a bot will chase a single target without reaching melee
/// before giving up and re-rolling. Two bots running at the same speed
/// in a chase loop will otherwise sprint at each other forever; this
/// kicks them back into the random-target rotation. Short enough that
/// a chase-loop doesn't drag bots across the whole arena before the
/// boundary clamp kicks in.
const PVP_CHASE_TIMEOUT: Duration = Duration::from_secs(3);

pub struct MovementDriver {
    info: MovementInfo,
    phase: Phase,
    phase_ends_at: Instant,
    last_heartbeat_at: Instant,
    started_at: Instant,
    metrics: Arc<Metrics>,
    mode: Mode,
    /// PvP only: monotonic clock at which we'll consider issuing the
    /// next swing — guards against firing faster than `UNARMED_SPEED`.
    next_swing_at: Instant,
    /// PvP only, pre-battle: random point inside `PVP_GATHER_RADIUS` of
    /// `ANCHOR` that the bot strafes toward during the gather phase.
    /// `None` until the first gather tick rolls it.
    gather_destination: Option<Vector3d>,
    /// PvP only: when the current pursuit started. `Some(t)` while
    /// running at a target; `None` while in melee or with no target.
    /// Used to bail out of equal-speed chase loops by dropping the
    /// target after `PVP_CHASE_TIMEOUT`.
    chase_started_at: Option<Instant>,
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
            next_swing_at: now,
            gather_destination: None,
            chase_started_at: None,
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
            Mode::Race { .. } => self.tick_race(writer, encrypter, now).await,
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

    /// Race tick. Three sub-phases:
    /// 1. First tick after world-enter: snap our position to `path[0] +
    ///    jitter`, flag `teleported`, send a single heartbeat. The
    ///    server trusts the new position and routes us into the
    ///    Booty Bay region next tick. Real anticheat would reject
    ///    this; the loadtest server doesn't run one.
    /// 2. Steady-state running: orient toward the current waypoint
    ///    (target = `path[index] + jitter`), advance forward at
    ///    `RUN_SPEED`, emit `MSG_MOVE_START_FORWARD_Client` on first
    ///    entry to a new waypoint and `MSG_MOVE_HEARTBEAT_Client`
    ///    every `HEARTBEAT_INTERVAL` afterward.
    /// 3. Arrival at the current waypoint: advance `index`. If we
    ///    reached the endpoint, flip `forward` and reverse the run.
    async fn tick_race(
        &mut self,
        writer: &mut OwnedWriteHalf,
        encrypter: &mut EncrypterHalf,
        now: Instant,
    ) -> std::io::Result<()> {
        // Snapshot mutable race fields up front so we can do all the
        // math against locals (no double-borrow of `self.mode`).
        let (path, mut index, mut forward, jx, jy, mut teleported) = match &self.mode {
            Mode::Race { path, index, forward, jitter, teleported } => {
                (path.clone(), *index, *forward, jitter.0, jitter.1, *teleported)
            }
            _ => unreachable!("tick_race called outside Mode::Race"),
        };
        if path.is_empty() {
            // No path configured — race mode with `--mode race` but no
            // waypoints set up (worker bug). Stand still so the bot
            // doesn't disconnect; operator-visible no-op.
            return Ok(());
        }

        // First-tick teleport. Override our position to `path[0] +
        // jitter`, emit a single heartbeat, and bail until next tick.
        if !teleported {
            let p0 = path[0];
            self.info.position = Vector3d {
                x: p0.x + jx,
                y: p0.y + jy,
                z: p0.z,
            };
            // Mark moving forward so observer clients interpolate
            // correctly between the teleport-heartbeat and the next
            // one. The actual MSG_MOVE_START_FORWARD goes out on the
            // following tick once we have an orientation.
            self.info.flags = MovementInfo_MovementFlags::empty();
            self.phase = Phase::Idle;
            let msg = MSG_MOVE_HEARTBEAT_Client {
                info: self.info.clone(),
            };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter).await?;
            writer.flush().await?;
            self.metrics.messages_out.fetch_add(1, Ordering::Relaxed);
            self.last_heartbeat_at = now;
            teleported = true;
            if let Mode::Race { teleported: t, .. } = &mut self.mode {
                *t = teleported;
            }
            return Ok(());
        }

        // Target waypoint with per-bot jitter applied.
        let target = Vector3d {
            x: path[index].x + jx,
            y: path[index].y + jy,
            z: path[index].z,
        };

        let dx = target.x - self.info.position.x;
        let dy = target.y - self.info.position.y;
        let dist_sq = dx * dx + dy * dy;

        // Arrival → advance index. If we've reached the endpoint in
        // the current direction, flip and reverse.
        if dist_sq <= RACE_WAYPOINT_RADIUS * RACE_WAYPOINT_RADIUS {
            if forward {
                if index + 1 >= path.len() {
                    forward = false;
                    // index stays at len-1; next tick will compute
                    // target = path[len-1] which we're already on, so
                    // we'll fall through into the decrement branch.
                } else {
                    index += 1;
                }
            } else if index == 0 {
                forward = true;
            } else {
                index -= 1;
            }
            // Write back so subsequent ticks pick up the advance.
            if let Mode::Race {
                index: i, forward: f, ..
            } = &mut self.mode
            {
                *i = index;
                *f = forward;
            }
            // Don't actually move this tick — let the next tick orient
            // fresh against the new waypoint.
            return Ok(());
        }

        let new_orientation = dy.atan2(dx);

        // Emit START_FORWARD when orientation drifts or we were idle.
        // The server resets its movement model on each START_*, so
        // observer-side extrapolation snaps to the new heading.
        let needs_start = !matches!(self.phase, Phase::Forward)
            || (self.info.orientation - new_orientation).abs() > 0.15;
        if needs_start {
            self.info.orientation = new_orientation;
            self.info.flags = MovementInfo_MovementFlags::new_forward();
            self.phase = Phase::Forward;
            let msg = MSG_MOVE_START_FORWARD_Client {
                info: self.info.clone(),
            };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter).await?;
            self.metrics.messages_out.fetch_add(1, Ordering::Relaxed);
        }

        if now.duration_since(self.last_heartbeat_at) >= HEARTBEAT_INTERVAL {
            self.advance_position(now);
            let msg = MSG_MOVE_HEARTBEAT_Client {
                info: self.info.clone(),
            };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter).await?;
            self.metrics.messages_out.fetch_add(1, Ordering::Relaxed);
            self.last_heartbeat_at = now;
        }

        writer.flush().await?;
        Ok(())
    }

    /// PvP tick (battle phase). Behavior:
    /// - Dead → fully inert. No movement, no swings, no target switching.
    ///   The bot's body stays where it fell.
    /// - Alive without target → ask `PvpState` to acquire one. If the
    ///   position cache is empty (very early in the battle) we just stand
    ///   still and wait for another bot's broadcasts to populate it.
    /// - Alive with target, target's position unknown → release the
    ///   stale lock (target probably died or wandered out of AOI) and
    ///   pick again next tick.
    /// - Alive with target in attack range → stop moving, swing on the
    ///   `UNARMED_SPEED` cadence.
    /// - Alive with target out of range → run a straight line toward
    ///   the target's last known position. Pure 2D euclidean — no
    ///   pathfinding, no collision. Open arena, no walls in the way.
    ///
    /// The target lock is dropped by `PvpState::record_attack_seen` once
    /// 100+ damage has landed on the target (across all attackers), so
    /// we never sit on a dead corpse waiting.
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

        // One critical section: acquire-target-if-needed + position
        // lookup + dead/alive snapshot. Releases the mutex before any
        // .await so the reader task can keep updating the cache.
        let (is_dead, target_guid, target_pos) = {
            let (state, own_guid) = match &self.mode {
                Mode::Pvp { state, own_guid, .. } => (state, *own_guid),
                _ => unreachable!("tick_pvp called outside Mode::Pvp"),
            };
            let mut state = state.lock().expect("pvp state mutex poisoned");
            let is_dead = state.last_death_at.is_some();
            if !is_dead {
                state.release_stale_target();
                state.acquire_target_if_needed(own_guid);
            }
            let target_guid = state.current_target;
            let mut target_pos = target_guid.and_then(|g| state.position_of(g));
            // Retaliation bootstrap: if we just acquired an attacker we
            // haven't seen broadcast a movement opcode yet (e.g. they
            // engaged from a stationary gather pose), stamp them at
            // our own position. The server-side melee range check
            // guarantees they're within ~7.5 yd to have landed a hit,
            // so own-pos is a safe upper bound. Their next movement
            // broadcast overwrites this with the real value.
            if let (Some(g), None) = (target_guid, target_pos) {
                state.observe(g, self.info.position);
                target_pos = Some(self.info.position);
            }
            (is_dead, target_guid, target_pos)
        };

        // Dead → corpse. Send nothing; let observers see the body where
        // it fell. The server has already broadcast the dead stand-state,
        // so other clients render the death pose.
        if is_dead {
            self.chase_started_at = None;
            return Ok(());
        }

        // No target / target's last-known position unknown → stand still.
        // The reader is constantly updating the cache from inbound move
        // packets, so this resolves on its own as soon as something
        // visible is broadcasting.
        let (Some(target_guid), Some(target_pos)) = (target_guid, target_pos) else {
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
        };

        // Target has wandered outside the arena — drop the lock and
        // pick someone else next tick. Without this a chaser would
        // happily follow an escapee, hit its own arena boundary, and
        // stall there forever sending heartbeats at the wall.
        let target_drift_sq =
            (target_pos.x - ANCHOR.x).powi(2) + (target_pos.y - ANCHOR.y).powi(2);
        if target_drift_sq > PVP_ARENA_RADIUS * PVP_ARENA_RADIUS {
            if let Mode::Pvp { state, .. } = &self.mode
                && let Ok(mut state) = state.lock()
            {
                state.drop_target();
            }
            self.chase_started_at = None;
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

        let dx = target_pos.x - self.info.position.x;
        let dy = target_pos.y - self.info.position.y;
        let dist_sq = dx * dx + dy * dy;
        let in_melee = dist_sq <= PVP_ATTACK_RANGE * PVP_ATTACK_RANGE;
        let new_orientation = dy.atan2(dx);

        // Chase-loop bail-out. Two bots at the same run speed will never
        // converge if both keep moving — the pursuer just stays at a
        // constant distance behind the target. After
        // `PVP_CHASE_TIMEOUT` of running at this target without ever
        // reaching melee, drop the lock and pick a fresh one. Reset the
        // clock as soon as we enter melee so the timer doesn't fire
        // mid-fight.
        if in_melee {
            self.chase_started_at = None;
        } else {
            let started = self.chase_started_at.get_or_insert(now);
            if now.duration_since(*started) >= PVP_CHASE_TIMEOUT {
                self.chase_started_at = None;
                if let Mode::Pvp { state, .. } = &self.mode
                    && let Ok(mut state) = state.lock()
                {
                    state.drop_target();
                }
                // No actions this tick — fall out so next tick picks a
                // new target. Stops the bot from continuing to sprint
                // in the now-stale direction.
                if !matches!(self.phase, Phase::Idle) {
                    self.info.flags = MovementInfo_MovementFlags::empty();
                    self.phase = Phase::Idle;
                    let msg = MSG_MOVE_STOP_Client {
                        info: self.info.clone(),
                    };
                    msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                        .await?;
                    self.metrics
                        .messages_out
                        .fetch_add(1, Ordering::Relaxed);
                }
                writer.flush().await?;
                return Ok(());
            }
        }

        if in_melee {
            // Stop pursuing and just stand there swinging. Face the
            // target so the visual is right (no big deal mechanically —
            // the server doesn't check facing for melee hits).
            if !matches!(self.phase, Phase::Idle)
                || (self.info.orientation - new_orientation).abs() > 0.15
            {
                self.info.orientation = new_orientation;
                self.info.flags = MovementInfo_MovementFlags::empty();
                self.phase = Phase::Idle;
                let msg = MSG_MOVE_STOP_Client {
                    info: self.info.clone(),
                };
                msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                    .await?;
                self.metrics
                    .messages_out
                    .fetch_add(1, Ordering::Relaxed);
            }
        } else {
            // Out of range — run forward at the target. Re-issue
            // START_FORWARD when our heading drifts or we were stopped;
            // the server resets its movement model on each START_*.
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
        }

        // Swing rate-cap: never send `CMSG_ATTACKSWING` faster than the
        // server would actually resolve it. The server still gates on
        // its own `UNARMED_SPEED` timer; this just avoids wasted bytes.
        if in_melee && now >= self.next_swing_at {
            let msg = CMSG_ATTACKSWING { guid: target_guid };
            msg.tokio_write_encrypted_client(&mut *writer, encrypter)
                .await?;
            self.metrics
                .messages_out
                .fetch_add(1, Ordering::Relaxed);
            self.next_swing_at = now + PVP_SWING_INTERVAL;
        }

        writer.flush().await?;
        Ok(())
    }

    /// Pre-battle gather phase. Each bot picks a random point inside
    /// `PVP_GATHER_RADIUS` of the arena center and strafes to it.
    /// Strafing rather than running so the bots look like they're
    /// shuffling into position — minor cosmetic, but the user asked
    /// for it. Bots stand still once arrived; the gather phase ends
    /// when the worker flips `battle_started`.
    async fn tick_gather(
        &mut self,
        writer: &mut OwnedWriteHalf,
        encrypter: &mut EncrypterHalf,
        now: Instant,
    ) -> std::io::Result<()> {
        if self.gather_destination.is_none() {
            // Uniform on radius (NOT on disc area). Uniform-on-area
            // would bias most bots toward the rim — "anywhere in
            // between" reads more naturally as "every radius equally
            // likely", which clusters bots near the center where
            // initial chases stay short.
            let (angle, r) = {
                let mut rng = rand::rng();
                let a = rng.random_range(0.0_f32..std::f32::consts::TAU);
                let r = rng.random_range(0.0_f32..PVP_GATHER_RADIUS);
                (a, r)
            };
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

    /// Move the puppet along its current orientation. Both modes clamp
    /// outward drift, but with different geometries:
    /// - `Mode::Random` bounces off a 60 yd box around the spawn anchor
    ///   by snapping back to anchor (cheap virtual-wall reflection).
    /// - `Mode::Pvp` refuses to advance once the proposed position would
    ///   leave `PVP_ARENA_RADIUS`. The bot effectively sticks at the
    ///   boundary, sending heartbeats with an unchanged position until
    ///   its target acquisition picks something inside the arena and
    ///   the orientation flips inward.
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
        let new_x = self.info.position.x + dx;
        let new_y = self.info.position.y + dy;

        if matches!(self.mode, Mode::Pvp { .. }) {
            // Hard arena boundary, outward-only. A bot that gathered
            // past the rim (gather radius is wider than the clamp) can
            // still walk INWARD — we only refuse advances that exit the
            // boundary AND move further from center than where we
            // already are. Without the outward-only test, anyone
            // gathered at radius 25+ would be unable to move at all.
            let new_drift_sq =
                (new_x - ANCHOR.x).powi(2) + (new_y - ANCHOR.y).powi(2);
            let old_drift_sq = (self.info.position.x - ANCHOR.x).powi(2)
                + (self.info.position.y - ANCHOR.y).powi(2);
            if new_drift_sq > PVP_ARENA_RADIUS * PVP_ARENA_RADIUS
                && new_drift_sq > old_drift_sq
            {
                return;
            }
        }

        self.info.position.x = new_x;
        self.info.position.y = new_y;

        if matches!(self.mode, Mode::Random) {
            let drift = ((self.info.position.x - ANCHOR.x).powi(2)
                + (self.info.position.y - ANCHOR.y).powi(2))
            .sqrt();
            if drift > MAX_DRIFT_YARDS {
                // Bounce: reset to anchor and pick a new heading; cheaper
                // than reflecting against a virtual wall.
                self.info.position = ANCHOR;
            }
        }
    }
}
