//! Server-side melee combat helpers — distance check, movement leeway.
//!
//! Faithful to cmangos classic (`Unit::CanReachWithMeleeAttack` in
//! `mangos-classic-master/src/game/Entities/Unit.cpp`):
//!
//! ```text
//! combined_reach = attacker.combat_reach + target.combat_reach
//!                + BASE_MELEERANGE_OFFSET (1.333)
//! if combined_reach < ATTACK_DISTANCE (5.0):
//!     combined_reach = ATTACK_DISTANCE
//! if attacker.is_moving && !is_walking
//!     && target.is_moving   && !is_walking:
//!     combined_reach += MELEE_LEEWAY (8/3)
//! ```
//!
//! Distance is 3D Euclidean — cmangos uses 3D for player-initiated melee
//! (NPC-initiated melee uses 2D). All swings in our codebase today are
//! player-initiated (real or bot), so we take the player path.

use wow_world_messages::vanilla::{MovementInfo, Vector3d};

/// cmangos `ATTACK_DISTANCE`. Floor on combined melee reach — even with
/// sub-standard reach values the pair can still hit at 5 yd.
pub const ATTACK_DISTANCE: f32 = 5.0;

/// cmangos `BASE_MELEERANGE_OFFSET`. Static cushion added to the sum of
/// both parties' `combat_reach`. cmangos's source comment notes the
/// "measured values in (1.3224, 1.342)" range.
pub const BASE_MELEERANGE_OFFSET: f32 = 1.333;

/// cmangos `MELEE_LEEWAY` — literally `8.0 / 3.0`. Added when *both*
/// attacker and target are moving (not walking) — compensates for the
/// up-to-13yd combined position-staleness from 250-500ms heartbeat gaps
/// at run speed. cmangos uses the strict "both moving" semantics; we
/// follow suit for parity.
pub const MELEE_LEEWAY: f32 = 8.0 / 3.0;

/// Default `combat_reach` for a humanoid player. cmangos sources this
/// from `CreatureModelInfo` DBC; for our flat 1.5 it matches the vanilla
/// humanoid model row.
pub const PLAYER_COMBAT_REACH: f32 = 1.5;

/// Hardcoded creature reach for now. Per-creature `combat_reach` from
/// mangos `creature_template.CombatReach` is a follow-up — currently
/// PvE melee isn't exercised so a flat 1.5 keeps the math symmetric.
pub const CREATURE_COMBAT_REACH: f32 = 1.5;

/// True if any directional flag is set. Walking is treated identically
/// to running — we don't have a /walk distinction in the loadtest path,
/// and bots always run. cmangos's "not walking" guard is therefore moot
/// here; if/when /walk arrives we'd intersect with `!get_walk_mode()`.
pub fn is_moving(info: &MovementInfo) -> bool {
    let f = &info.flags;
    f.get_forward() || f.get_backward() || f.get_strafe_left() || f.get_strafe_right()
}

/// Combined melee reach in yards. Mirrors cmangos's
/// `GetCombinedCombatReach` plus the moving-leeway adjustment.
pub fn melee_range_yards(
    attacker_moving: bool,
    target_moving: bool,
    target_is_creature: bool,
) -> f32 {
    let target_reach = if target_is_creature {
        CREATURE_COMBAT_REACH
    } else {
        PLAYER_COMBAT_REACH
    };
    let mut reach = PLAYER_COMBAT_REACH + target_reach + BASE_MELEERANGE_OFFSET;
    if reach < ATTACK_DISTANCE {
        reach = ATTACK_DISTANCE;
    }
    if attacker_moving && target_moving {
        reach += MELEE_LEEWAY;
    }
    reach
}

/// 3D Euclidean squared distance. Matches cmangos's player-attacker
/// path. Squared to avoid the sqrt — caller compares against
/// `range * range`.
pub fn distance_sq_3d(a: &Vector3d, b: &Vector3d) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    dx * dx + dy * dy + dz * dz
}

#[cfg(test)]
mod tests {
    use super::*;
    use wow_world_messages::vanilla::MovementInfo_MovementFlags;

    fn info_with(flags: MovementInfo_MovementFlags) -> MovementInfo {
        MovementInfo {
            flags,
            timestamp: 0,
            position: Vector3d { x: 0.0, y: 0.0, z: 0.0 },
            orientation: 0.0,
            fall_time: 0.0,
        }
    }

    #[test]
    fn melee_range_neither_moving_floors_at_5yd() {
        // 1.5 + 1.5 + 1.333 = 4.333 → floored to 5.0
        assert_eq!(melee_range_yards(false, false, false), ATTACK_DISTANCE);
    }

    #[test]
    fn melee_range_one_moving_no_leeway() {
        // cmangos's strict "both moving" rule: one-side movement adds nothing.
        assert_eq!(melee_range_yards(true, false, false), ATTACK_DISTANCE);
        assert_eq!(melee_range_yards(false, true, false), ATTACK_DISTANCE);
    }

    #[test]
    fn melee_range_both_moving_adds_leeway() {
        let r = melee_range_yards(true, true, false);
        assert!((r - (ATTACK_DISTANCE + MELEE_LEEWAY)).abs() < 1e-5);
    }

    #[test]
    fn melee_range_creature_target_matches_player() {
        // Reach is 1.5 either way, so creature vs player target are equal.
        assert_eq!(
            melee_range_yards(false, false, true),
            melee_range_yards(false, false, false),
        );
        assert_eq!(
            melee_range_yards(true, true, true),
            melee_range_yards(true, true, false),
        );
    }

    #[test]
    fn is_moving_empty_flags_false() {
        assert!(!is_moving(&info_with(MovementInfo_MovementFlags::empty())));
    }

    #[test]
    fn is_moving_forward_flag_true() {
        assert!(is_moving(&info_with(MovementInfo_MovementFlags::new_forward())));
    }

    #[test]
    fn is_moving_backward_flag_true() {
        assert!(is_moving(&info_with(MovementInfo_MovementFlags::new_backward())));
    }

    #[test]
    fn is_moving_strafe_flags_true() {
        assert!(is_moving(&info_with(MovementInfo_MovementFlags::new_strafe_left())));
        assert!(is_moving(&info_with(MovementInfo_MovementFlags::new_strafe_right())));
    }

    #[test]
    fn is_moving_turn_only_false() {
        // Turning in place isn't translation — cmangos's `isMoving` would
        // include it via velocity, but our flag-based proxy intentionally
        // excludes it. Worst case the leeway doesn't trip for a turn-and-strike,
        // which matches the "you weren't actually moving" intuition.
        assert!(!is_moving(&info_with(MovementInfo_MovementFlags::new_turn_left())));
        assert!(!is_moving(&info_with(MovementInfo_MovementFlags::new_turn_right())));
    }

    #[test]
    fn distance_sq_3d_zero_when_equal() {
        let v = Vector3d { x: 1.0, y: 2.0, z: 3.0 };
        assert_eq!(distance_sq_3d(&v, &v), 0.0);
    }

    #[test]
    fn distance_sq_3d_includes_z() {
        let a = Vector3d { x: 0.0, y: 0.0, z: 0.0 };
        let b = Vector3d { x: 0.0, y: 0.0, z: 3.0 };
        assert_eq!(distance_sq_3d(&a, &b), 9.0);
    }

    #[test]
    fn distance_sq_3d_pythagorean() {
        let a = Vector3d { x: 0.0, y: 0.0, z: 0.0 };
        let b = Vector3d { x: 3.0, y: 4.0, z: 0.0 };
        assert_eq!(distance_sq_3d(&a, &b), 25.0);
    }
}
