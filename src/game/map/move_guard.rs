//! What a client may say of its own player's movement, past `finite`: as
//! fast as its speed allows over time, not per packet; nowhere while a root
//! or stun holds it; and with no flag it has no aura or order for (mangos-
//! classic's `VerifyMovementInfo`, and its anticheat's speed tests and
//! `VerifyMovementFlags`).

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::*;

use wow_world_messages::Guid;

use crate::config::AnticheatConfig;
use crate::game::conn::{Order, Outbound};
use crate::game::map::auras::{AURA_FEATHER_FALL, AURA_HOVER, AURA_MOD_ROOT, AURA_MOD_STUN, Auras};
use crate::game::map::player::Player;
use crate::game::update::Speeds;

/// A player's movement check, between packets.
#[derive(Debug, Default, Clone, Copy)]
pub struct MoveGuard {
    /// Yards the player may still go ahead of its speed (none: all of
    /// them). Time fills it, at the speed and tolerance, up to
    /// `latency_allowance_secs` of that and `distance_allowance_yards`;
    /// every move spends it, so sending more packets gains nothing.
    bank: Option<f32>,
    /// Since when a root or stun has been seen holding the player.
    held_since: Option<Instant>,
    /// A knockback taken and not yet landed: its horizontal speed, and
    /// when (mangos's `IsLaunched`, `Movement::KnockBack`).
    launch: Option<(f32, Instant)>,
}

impl MoveGuard {
    /// The bank refilled: the unit was put somewhere by the server.
    pub fn refill(&mut self) {
        self.bank = None;
    }
}

/// How long a client may go on moving under a root or stun whose order it
/// hasn't acknowledged: the order is on its way (mangos's pending root).
pub const ROOT_GRACE: Duration = Duration::from_secs(1);

/// The longest a knockback's flight is allowed for, if the client never
/// says it landed.
pub const LAUNCH_LIMIT: Duration = Duration::from_secs(10);

/// How far a held player's client may say it is from where the server
/// holds it, horizontally, before its packet is refused: a fall or the
/// client's float arithmetic, no more (wvs's dead body's half yard).
const HELD_SLACK: f32 = 0.5;

/// What the checks look at of the unit a client moves: its own player, or
/// a unit it possesses (mangos-classic's `VerifyMovementInfo` checks the
/// mover, whoever it is).
pub struct Mover<'a> {
    pub guid: Guid,
    pub name: &'a str,
    /// Where the server has it.
    pub position: Vector3d,
    pub auras: &'a Auras,
    pub speeds: Speeds,
    /// The client its orders (root, water walk, hover...) wait on.
    pub orders: &'a Outbound,
    /// A GM's flags aren't taken off.
    pub gm: bool,
}

impl<'a> Mover<'a> {
    /// A player their own client moves.
    pub fn player(p: &'a Player) -> Self {
        Self::player_moved_by(p, &p.outbound)
    }

    /// A player the client of `orders` moves (theirs, or a possessor's).
    pub fn player_moved_by(p: &'a Player, orders: &'a Outbound) -> Self {
        Self {
            guid: p.guid,
            name: &p.name,
            position: p.position(),
            auras: &p.auras,
            speeds: p.speeds,
            orders,
            gm: p.gm,
        }
    }
}

/// Whether a root or stun holds `p` (mangos's `UNIT_STAT_ROOT`,
/// `UNIT_STAT_STUNNED`).
pub fn held(p: &Player) -> bool {
    held_auras(&p.auras)
}

fn held_auras(auras: &Auras) -> bool {
    auras.has(AURA_MOD_ROOT) || auras.has(AURA_MOD_STUN)
}

/// Whether the client's packet tries to go somewhere while a root or stun
/// holds the player, past the grace its order has to arrive in (mangos's
/// `VerifyMovementInfo`: a rooted mover's moving packet is ignored). A
/// launch in flight goes on, and a held player still falls: only steering
/// or a step across is refused.
pub fn held_back(p: &mut Player, info: &MovementInfo, now: Instant) -> bool {
    let mut guard = p.misc.move_guard;
    let held = held_back_with(p, &mut guard, info, now);
    p.misc.move_guard = guard;
    held
}

/// [`held_back`], with the guard `guard` (the player's own, or a copy).
pub fn held_back_with(
    p: &Player,
    guard: &mut MoveGuard,
    info: &MovementInfo,
    now: Instant,
) -> bool {
    held_back_of(&Mover::player(p), guard, info, now)
}

/// [`held_back_with`], of any mover.
pub fn held_back_of(p: &Mover, guard: &mut MoveGuard, info: &MovementInfo, now: Instant) -> bool {
    if !held_auras(p.auras) {
        guard.held_since = None;
        return false;
    }
    let since = *guard.held_since.get_or_insert(now);
    if guard.launch.is_some() {
        return false;
    }
    let g = p.guid;
    let pending = p.orders.pending_orders().iter().any(|(_, o)| {
        *o == Order::Root {
            guid: g,
            root: true,
        }
    });
    if pending && now.duration_since(since) < ROOT_GRACE {
        return false;
    }
    let f = &info.flags;
    let steering = f.get_forward()
        || f.get_backward()
        || f.get_strafe_left()
        || f.get_strafe_right()
        || f.get_pitch_up()
        || f.get_pitch_down();
    steering || horizontal(p.position, info.position) > HELD_SLACK
}

/// `info` with the flags the player has no right to taken off, as mangos's
/// `VerifyMovementFlags` takes them (strict): the old fly hack's mask,
/// swimming and flying at once, levitating, being able to fly, and
/// water walking, slow falling or hovering with no aura for it and no order
/// taking it away on its way. wvs takes flying alone off too (no 1.12
/// player flies but on a flight's spline) and hovering (mangos's "things to
/// check: hover"). Not for a GM, nor in a launch's flight (mangos: "maybe
/// false positives").
pub fn stripped(p: &Player, info: MovementInfo, walks_on_water: bool) -> MovementInfo {
    stripped_of(&Mover::player(p), &p.misc.move_guard, info, walks_on_water)
}

/// [`stripped`], of any mover, with its guard.
pub fn stripped_of(
    p: &Mover,
    guard: &MoveGuard,
    info: MovementInfo,
    walks_on_water: bool,
) -> MovementInfo {
    if p.gm || guard.launch.is_some() {
        return info;
    }
    let mut f = info.flags;
    let was = f;
    if f.get_walk_mode()
        && f.get_swimming().is_some()
        && f.get_hover()
        && f.get_fallingfar()
        && f.get_flying()
        && f.get_on_transport().is_some()
    {
        f = MovementInfo_MovementFlags::empty();
    }
    if f.get_swimming().is_some() && f.get_flying() {
        f = f.clear_swimming().clear_flying();
    }
    f = f.clear_levitating().clear_can_fly().clear_flying();
    let g = p.guid;
    let pending = |off: Order| p.orders.pending_orders().iter().any(|(_, o)| *o == off);
    if f.get_waterwalking() && !walks_on_water && !pending(Order::WaterWalk { guid: g, on: false })
    {
        f = f.clear_waterwalking();
    }
    if f.get_safe_fall()
        && !p.auras.has(AURA_FEATHER_FALL)
        && !pending(Order::FeatherFall { guid: g, on: false })
    {
        f = f.clear_safe_fall();
    }
    if f.get_hover() && !p.auras.has(AURA_HOVER) && !pending(Order::Hover { guid: g, on: false }) {
        f = f.clear_hover();
    }
    if f != was {
        tracing::debug!(
            "{} sent movement flags it has no right to; taken off",
            p.name
        );
    }
    MovementInfo { flags: f, ..info }
}

/// Whether the move from `last` (taken at `at`) to `info` is further than
/// the player could go: its speed (the faster of run and swim, slows and
/// all; a launch's, in its flight) at the tolerance, for the time since,
/// drawn from what is banked. A move within it spends the bank.
pub fn too_fast(
    p: &mut Player,
    last: Vector3d,
    at: Instant,
    info: &MovementInfo,
    now: Instant,
    check: &AnticheatConfig,
) -> bool {
    let mut guard = p.misc.move_guard;
    let fast = too_fast_with(p, &mut guard, last, at, info, now, check);
    p.misc.move_guard = guard;
    fast
}

/// [`too_fast`], with the guard `guard` (the player's own, or a copy).
pub fn too_fast_with(
    p: &Player,
    guard: &mut MoveGuard,
    last: Vector3d,
    at: Instant,
    info: &MovementInfo,
    now: Instant,
    check: &AnticheatConfig,
) -> bool {
    too_fast_at(p.speeds, guard, last, at, info, now, check)
}

/// [`too_fast_with`], at the speeds `speeds`.
pub fn too_fast_at(
    speeds: Speeds,
    guard: &mut MoveGuard,
    last: Vector3d,
    at: Instant,
    info: &MovementInfo,
    now: Instant,
    check: &AnticheatConfig,
) -> bool {
    if let Some((_, since)) = guard.launch
        && now.duration_since(since) > LAUNCH_LIMIT
    {
        guard.launch = None;
    }
    let launch = guard.launch.map_or(0.0, |(xy, _)| xy);
    let speed = speeds.run.max(speeds.swim).max(launch).max(0.0);
    let rate = speed * check.speed_tolerance;
    let cap = rate * check.latency_allowance_secs + check.distance_allowance_yards;
    let dt = now.saturating_duration_since(at).as_secs_f32();
    let bank = (guard.bank.unwrap_or(cap) + rate * dt).min(cap);
    let d = horizontal(last, info.position);
    if d > bank {
        guard.bank = None;
        return true;
    }
    guard.bank = Some(bank - d);
    false
}

/// The bank refilled: the player was put somewhere by the server.
pub fn refill(p: &mut Player) {
    p.misc.move_guard.bank = None;
}

/// A knockback the client took: until it lands, it goes at the launch's
/// speed, and its bank is full at it.
pub fn launched(p: &mut Player, xy_speed: f32, now: Instant) {
    launched_with(&mut p.misc.move_guard, xy_speed, now);
}

/// [`launched`], with the guard `guard`.
pub fn launched_with(guard: &mut MoveGuard, xy_speed: f32, now: Instant) {
    guard.launch = Some((xy_speed.abs(), now));
    guard.bank = None;
}

/// A move taken: a packet on the ground or in the water ends a launch
/// (mangos's `SetLaunched(false)` at `MSG_MOVE_FALL_LAND` and
/// `MSG_MOVE_START_SWIM`).
pub fn taken(p: &mut Player, info: &MovementInfo) {
    taken_with(&mut p.misc.move_guard, info);
}

/// [`taken`], with the guard `guard`.
pub fn taken_with(guard: &mut MoveGuard, info: &MovementInfo) {
    let f = &info.flags;
    let aloft = f.get_jumping().is_some() || f.get_fallingfar();
    if !aloft || f.get_swimming().is_some() {
        guard.launch = None;
    }
}

fn horizontal(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// As if the root or stun on `p` had been seen `by` earlier: a test's way
/// past [`ROOT_GRACE`] without waiting it out.
#[cfg(test)]
pub fn backdate_held(p: &mut Player, by: Duration) {
    let guard = &mut p.misc.move_guard;
    guard.held_since = guard.held_since.and_then(|t| t.checked_sub(by));
}
