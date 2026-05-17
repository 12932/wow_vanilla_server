//! Shared PvP state for a single bot.
//!
//! The bot's `read_fut` feeds inbound packets into this struct; the
//! `MovementDriver` (on the same task) reads it to decide what to do.
//! Wrapped in `Arc<Mutex<>>` because the two futures share the task but
//! we want non-async access.
//!
//! Three things live here:
//! 1. **Seen-position cache.** Every `MSG_MOVE_*_Server` packet observed
//!    by the reader inserts `(guid → last known position)`. The driver
//!    consults this when acquiring a target and when chasing one.
//! 2. **Self-death accounting.** Inbound combat log packets where
//!    `target == own_guid` sum into `damage_taken`; crossing
//!    `PVP_MAX_HEALTH = 100` flips `last_death_at`. Once dead the bot
//!    is inert (no movement, no swings, no target switching) — under
//!    the current PvP rules there's no respawn, dead bots stay as
//!    corpses where they fell.
//! 3. **Target lock.** Once a target is acquired the bot sticks with
//!    them until the target dies. We track damage seen against the
//!    target (sum of all attackers, since whoever lands the killing
//!    blow is fine by us) and drop the lock when it crosses 100 OR
//!    when the target's seen-position entry expires (target stopped
//!    moving — likely dead or out of AOI).
//!
//! Only allocated when the worker is started with `--pvp`. Without that
//! flag the bot doesn't reference any of this — the `Mode::Random`
//! driver path is unaffected.

use ahash::AHashMap;
use rand::seq::IteratorRandom;
use std::time::{Duration, Instant};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

/// Entries older than this are pruned at pick time. Stale targets aren't
/// worth chasing — by the time we'd reach the last-known position, the
/// real owner has long since moved on, died, or left the AOI. The driver
/// also uses "target's entry is gone from the cache" as an implicit
/// signal that the target is dead/unreachable and drops the lock.
const STALE_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct PvpState {
    seen: AHashMap<Guid, (Vector3d, Instant)>,
    /// Damage applied to *us* since spawn. Compared against
    /// `PVP_MAX_HEALTH` to decide that we've died. We stop accumulating
    /// once dead so a flood of subsequent corpse-hits doesn't matter.
    pub damage_taken: u32,
    /// Set when `damage_taken` first crosses the death threshold. The
    /// driver checks this each tick to gate all activity.
    pub last_death_at: Option<Instant>,
    /// Locked target. Set by [`acquire_target_if_needed`], cleared by
    /// [`record_attack_seen`] when the target has taken enough cumulative
    /// damage, or by the driver if the target's seen-position entry has
    /// expired.
    pub current_target: Option<Guid>,
    /// Total damage we've observed landing on `current_target`, summed
    /// across every attacker. Crossing `PVP_MAX_HEALTH` drops the lock.
    pub damage_dealt_to_target: u32,
}

impl PvpState {
    pub fn observe(&mut self, guid: Guid, pos: Vector3d) {
        self.seen.insert(guid, (pos, Instant::now()));
    }

    pub fn position_of(&self, guid: Guid) -> Option<Vector3d> {
        self.seen.get(&guid).map(|(p, _)| *p)
    }

    /// If we don't have a target yet, pick a random one from the
    /// position cache that isn't `exclude` (our own guid). Resets the
    /// damage-dealt counter for the new target. Stale entries get
    /// pruned in the same pass.
    pub fn acquire_target_if_needed(&mut self, exclude: Guid) {
        if self.current_target.is_some() {
            return;
        }
        let now = Instant::now();
        self.seen
            .retain(|_, (_, seen)| now.duration_since(*seen) < STALE_AFTER);
        let mut rng = rand::rng();
        let pick = self
            .seen
            .iter()
            .filter(|(g, _)| **g != exclude)
            .choose(&mut rng)
            .map(|(g, _)| *g);
        if let Some(g) = pick {
            self.current_target = Some(g);
            self.damage_dealt_to_target = 0;
        }
    }

    /// Drop the target lock if the target hasn't been seen recently.
    /// Called by the driver each tick so corpses (who broadcast no
    /// further movement) naturally fall out of the rotation.
    pub fn release_stale_target(&mut self) {
        if let Some(g) = self.current_target
            && !self.seen.contains_key(&g)
        {
            self.current_target = None;
            self.damage_dealt_to_target = 0;
        }
    }

    /// Reader hook for inbound `SMSG_ATTACKERSTATEUPDATE`. Accumulates
    /// damage against our own guid (for self-death detection) and
    /// against our current target (for target-death detection). Either
    /// path crossing `PVP_MAX_HEALTH` triggers the relevant state
    /// transition.
    pub fn record_attack_seen(&mut self, target: Guid, damage: u32, own_guid: Guid) {
        if target == own_guid {
            self.take_damage(damage);
            return;
        }
        if Some(target) == self.current_target {
            self.damage_dealt_to_target = self.damage_dealt_to_target.saturating_add(damage);
            if self.damage_dealt_to_target >= PVP_MAX_HEALTH {
                self.current_target = None;
                self.damage_dealt_to_target = 0;
            }
        }
    }

    fn take_damage(&mut self, amount: u32) {
        if self.last_death_at.is_some() {
            return;
        }
        self.damage_taken = self.damage_taken.saturating_add(amount);
        if self.damage_taken >= PVP_MAX_HEALTH {
            self.last_death_at = Some(Instant::now());
            self.current_target = None;
            self.damage_dealt_to_target = 0;
        }
    }
}

/// Mirrors the server-side `PVP_MAX_HEALTH` in
/// `src/world/world_opcode_handler/character.rs`. Keep in sync — if the
/// server lowers it, bots will think they died with HP still on the bar.
pub const PVP_MAX_HEALTH: u32 = 100;
