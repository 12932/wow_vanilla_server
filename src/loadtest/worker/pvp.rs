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
    /// expired / chase-timed-out.
    pub current_target: Option<Guid>,
    /// Total damage we've observed landing on `current_target`, summed
    /// across every attacker. Crossing `PVP_MAX_HEALTH` drops the lock.
    pub damage_dealt_to_target: u32,
    /// Most recent attacker (from inbound `SMSG_ATTACKERSTATEUPDATE`
    /// where `target == own_guid`). One-shot signal consumed by
    /// [`acquire_target_if_needed`] — set whenever a hit lands on us,
    /// taken when we next pick a target. Lets a bot retaliate against
    /// whoever just hit it instead of standing idle / rolling a random
    /// unrelated bot.
    pub last_attacker: Option<Guid>,
}

impl PvpState {
    pub fn observe(&mut self, guid: Guid, pos: Vector3d) {
        self.seen.insert(guid, (pos, Instant::now()));
    }

    pub fn position_of(&self, guid: Guid) -> Option<Vector3d> {
        self.seen.get(&guid).map(|(p, _)| *p)
    }

    /// If we don't have a target yet, pick one. Retaliation first:
    /// whoever last hit us (if anyone) takes priority over a random
    /// pick — `last_attacker` is consumed via `take()` so the same hit
    /// only re-acquires once, and the existing `current_target` lock
    /// keeps us glued to the attacker through subsequent hits. Falls
    /// back to a uniform-random sample over the seen-position cache.
    /// Stale `seen` entries are pruned in the same pass.
    pub fn acquire_target_if_needed(&mut self, exclude: Guid) {
        if self.current_target.is_some() {
            return;
        }
        let now = Instant::now();
        self.seen
            .retain(|_, (_, seen)| now.duration_since(*seen) < STALE_AFTER);

        if let Some(attacker) = self.last_attacker.take()
            && attacker != exclude
        {
            self.current_target = Some(attacker);
            self.damage_dealt_to_target = 0;
            return;
        }

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

    /// Force-drop the current target. Driver calls this when its
    /// chase-timeout fires (we've been running at the target for
    /// `PVP_CHASE_TIMEOUT` without ever reaching melee — almost
    /// certainly a same-speed chase loop with no convergence).
    pub fn drop_target(&mut self) {
        self.current_target = None;
        self.damage_dealt_to_target = 0;
    }

    /// Reader hook for inbound `SMSG_ATTACKERSTATEUPDATE`. Three jobs:
    /// 1. Self-damage (target == own_guid) → drives our death state and
    ///    flags `last_attacker` for retaliation on the next acquire.
    /// 2. Damage on `current_target` → drops the lock when their HP
    ///    bucket crosses `PVP_MAX_HEALTH`.
    /// 3. Anything else → ignored.
    pub fn record_attack_seen(
        &mut self,
        attacker: Guid,
        target: Guid,
        damage: u32,
        own_guid: Guid,
    ) {
        if target == own_guid {
            if self.last_death_at.is_some() {
                return;
            }
            self.take_damage(damage);
            // Skip self-hits and the zero guid (server-internal); only
            // real bots are worth retaliating against. take_damage may
            // have just killed us — `take_damage` clears the slot in
            // that case, so this set is a no-op when dead.
            if self.last_death_at.is_none()
                && attacker != Guid::zero()
                && attacker != own_guid
            {
                self.last_attacker = Some(attacker);
            }
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
            self.last_attacker = None;
        }
    }
}

/// Mirrors the server-side `PVP_MAX_HEALTH` in
/// `src/world/world_opcode_handler/character.rs`. Keep in sync — if the
/// server lowers it, bots will think they died with HP still on the bar.
pub const PVP_MAX_HEALTH: u32 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: Guid = Guid::new(0x1000);
    const ATTACKER: Guid = Guid::new(0x2000);
    const BYSTANDER: Guid = Guid::new(0x3000);

    fn pos(x: f32, y: f32) -> Vector3d {
        Vector3d { x, y, z: 0.0 }
    }

    #[test]
    fn record_attack_seen_sets_last_attacker_when_we_are_hit() {
        let mut s = PvpState::default();
        s.record_attack_seen(ATTACKER, OWN, 10, OWN);
        assert_eq!(s.last_attacker, Some(ATTACKER));
        assert_eq!(s.damage_taken, 10);
    }

    #[test]
    fn record_attack_seen_ignores_self_hits() {
        // Both attacker and target are us — shouldn't populate
        // last_attacker (we don't retaliate against ourselves).
        let mut s = PvpState::default();
        s.record_attack_seen(OWN, OWN, 5, OWN);
        assert_eq!(s.last_attacker, None);
    }

    #[test]
    fn record_attack_seen_ignores_zero_guid_attacker() {
        let mut s = PvpState::default();
        s.record_attack_seen(Guid::zero(), OWN, 5, OWN);
        assert_eq!(s.last_attacker, None);
    }

    #[test]
    fn acquire_target_prefers_last_attacker_over_random() {
        let mut s = PvpState::default();
        // Seen cache has bystander; attacker may or may not be in it
        // (here it isn't — covers the retaliation-against-not-yet-seen
        // path).
        s.observe(BYSTANDER, pos(10.0, 0.0));
        s.last_attacker = Some(ATTACKER);
        s.acquire_target_if_needed(OWN);
        assert_eq!(s.current_target, Some(ATTACKER));
        // One-shot: consumed.
        assert_eq!(s.last_attacker, None);
    }

    #[test]
    fn acquire_target_falls_back_to_random_when_no_attacker() {
        let mut s = PvpState::default();
        s.observe(BYSTANDER, pos(10.0, 0.0));
        s.acquire_target_if_needed(OWN);
        assert_eq!(s.current_target, Some(BYSTANDER));
    }

    #[test]
    fn acquire_target_ignores_attacker_equal_to_exclude() {
        // Defensive: even if last_attacker somehow got set to our own
        // guid, we shouldn't target ourselves. Falls through to the
        // random pick.
        let mut s = PvpState::default();
        s.observe(BYSTANDER, pos(10.0, 0.0));
        s.last_attacker = Some(OWN);
        s.acquire_target_if_needed(OWN);
        assert_eq!(s.current_target, Some(BYSTANDER));
    }

    #[test]
    fn acquire_target_noop_when_already_locked() {
        let mut s = PvpState {
            current_target: Some(BYSTANDER),
            last_attacker: Some(ATTACKER),
            ..Default::default()
        };
        s.acquire_target_if_needed(OWN);
        // Lock preserved; attacker slot untouched (will be honored
        // only after the current target is dropped).
        assert_eq!(s.current_target, Some(BYSTANDER));
        assert_eq!(s.last_attacker, Some(ATTACKER));
    }

    #[test]
    fn lethal_self_hit_clears_last_attacker() {
        let mut s = PvpState::default();
        s.record_attack_seen(ATTACKER, OWN, PVP_MAX_HEALTH, OWN);
        // Death cleared the slot — dead bots don't retaliate.
        assert!(s.last_death_at.is_some());
        assert_eq!(s.last_attacker, None);
    }

    #[test]
    fn post_death_hits_dont_repopulate_last_attacker() {
        let mut s = PvpState {
            last_death_at: Some(Instant::now()),
            ..Default::default()
        };
        s.record_attack_seen(ATTACKER, OWN, 10, OWN);
        assert_eq!(s.last_attacker, None);
        // Damage_taken unchanged — take_damage early-returns on dead.
        assert_eq!(s.damage_taken, 0);
    }
}
