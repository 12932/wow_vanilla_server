//! Stealth and invisibility, and who sees through them, as Thistle Tea's
//! `StealthDetection` and `Invisibility` follow vmangos: a stealthed unit
//! is seen by an enemy only within a distance that grows with the enemy's
//! level and detection and shrinks with the stealth's skill; an invisible
//! one only by those who detect its kind of invisibility well enough.
//! Friends see through both.

use wow_world_messages::Guid;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, dist};

const COLLISION_YARDS: f32 = 1.5;
const MAX_YARDS: f32 = 30.0;

/// The distance within which `viewer` (of `level`, a player or not) sees a
/// target stealthed at `stealth_skill`.
pub fn detection_distance(
    level: u32,
    player: bool,
    target_player: bool,
    target_level: u32,
    detection_bonus: i32,
    stealth_skill: i32,
    behind: bool,
) -> f32 {
    let creature = 5.0 / 6.0;
    let base = if player {
        if target_player { 9.0 } else { 21.0 }
    } else {
        creature
    };
    let per_level = if player { 1.5 } else { creature };
    let scale = if level as i32 - target_level as i32 > 3 {
        2.0
    } else {
        1.0
    };
    let skill = (level * 5) as i32 + detection_bonus - stealth_skill;
    let d = (base + skill as f32 * per_level * scale / 5.0).clamp(0.0, MAX_YARDS);
    d - if behind { 9.0 } else { 0.0 }
}

fn level(map: &Map, g: Guid) -> u32 {
    map.unit_view(g).map(|u| u.level).unwrap_or(1)
}

fn friendly(map: &Map, viewer: Guid, target: Guid) -> bool {
    if let (Some(a), Some(b)) = (map.players.get(&viewer), map.players.get(&target)) {
        return a.team == b.team && !crate::game::map::duel::dueling(map, viewer, target);
    }
    !combat::can_attack(map, viewer, target) && !combat::hostile(map, viewer, target)
}

/// Whether `viewer` sees `target` through its stealth or invisibility.
pub fn sees(map: &Map, viewer: Guid, target: Guid) -> bool {
    if viewer == target || !(guid::is_player(target) || guid::is_unit(target)) {
        return true;
    }
    let Some(a) = auras::auras(map, target) else {
        return true;
    };
    let stealthed = a.has(AURA_MOD_STEALTH);
    let invisible = a.has(AURA_MOD_INVISIBILITY);
    if !stealthed && !invisible {
        return true;
    }
    if friendly(map, viewer, target) {
        return true;
    }
    if map.players.get(&viewer).is_some_and(|p| {
        p.fields
            .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_GM)
    }) {
        return true;
    }
    let va = auras::auras(map, viewer);
    // Hunter's Mark and the like: the marker sees the marked.
    if a.holders
        .iter()
        .any(|h| h.caster == viewer && h.has(AURA_MOD_STALKED))
    {
        return true;
    }
    if invisible {
        let boss = map
            .creature(viewer)
            .and_then(|c| map.realm.data.creature_template(c.entry))
            .is_some_and(|t| t.rank == 3);
        let detected = boss
            || a.effects()
                .filter(|(_, x)| x.kind == AURA_MOD_INVISIBILITY)
                .any(|(_, x)| {
                    va.is_some_and(|v| {
                        v.effects().any(|(_, d)| {
                            (d.kind == AURA_MOD_INVISIBILITY && d.misc == x.misc)
                                || (d.kind == AURA_MOD_INVISIBILITY_DETECTION
                                    && d.misc == x.misc
                                    && d.amount >= x.amount)
                        })
                    })
                    // The drunk see drunken invisibility.
                    || (x.misc == crate::game::map::intoxication::DRUNK_INVISIBILITY
                        && crate::game::map::intoxication::value(map, viewer) as i32 >= x.amount)
                });
        if !detected {
            return false;
        }
    }
    if !stealthed {
        return true;
    }
    // A stunned viewer sees no one stealthed.
    if va.is_some_and(|v| v.has(AURA_MOD_STUN)) {
        return false;
    }
    let (Some(pv), Some(pt)) = (map.position_of(viewer), map.position_of(target)) else {
        return true;
    };
    let d = dist(pv, pt);
    if d < COLLISION_YARDS {
        return true;
    }
    let target_level = level(map, target);
    let skill =
        a.max(AURA_MOD_STEALTH).max((target_level * 5) as i32) + a.sum(AURA_MOD_STEALTH_LEVEL);
    let bonus = va.map_or(0, |v| v.sum_misc(AURA_MOD_STEALTH_DETECT, 0));
    let behind = map.unit_view(viewer).is_some_and(|u| {
        let angle = (pt.y - u.position.y).atan2(pt.x - u.position.x);
        let mut diff = (angle - u.orientation).rem_euclid(std::f32::consts::TAU);
        if diff > std::f32::consts::PI {
            diff = std::f32::consts::TAU - diff;
        }
        diff > std::f32::consts::FRAC_PI_2
    });
    d <= detection_distance(
        level(map, viewer),
        guid::is_player(viewer),
        guid::is_player(target),
        target_level,
        bonus,
        skill,
        behind,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_spot_a_stealthy_rogue_close_up() {
        // A level 60 player against a level 60 rogue at skill 300: nine
        // yards in front, none behind.
        let d = detection_distance(60, true, true, 60, 0, 300, false);
        assert!((d - 9.0).abs() < 0.01, "{d}");
        assert!(detection_distance(60, true, true, 60, 0, 300, true) <= 0.0);
        // More stealth, less distance; a creature sees less than a player.
        assert!(detection_distance(60, true, true, 60, 0, 315, false) < d);
        assert!(detection_distance(60, false, true, 60, 0, 300, false) < d);
    }
}
