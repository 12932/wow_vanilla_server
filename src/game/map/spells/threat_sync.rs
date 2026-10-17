//! Threat a unit holds for a while, as Thistle Tea's `TauntSync`,
//! `ThreatSync` and `Threat.set_temporary` keep it: a taunt aura borrows
//! the threat that brings its caster level with the creature's target,
//! given back when the caster's last taunt there ends (what was earned
//! meanwhile stays), while Taunt's own attack-me effect matches it for
//! good first; Fade's reduction goes on the creatures already fighting the
//! priest and comes off when it ends, and creatures met while faded don't
//! take it.

use wow_world_messages::Guid;

use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_MOD_TAUNT, AURA_MOD_TOTAL_THREAT};

/// `unit`'s threat on `creature` changed by `delta`, no lower than none.
fn modify(map: &mut Map, creature: Guid, unit: Guid, delta: f32) {
    if let Some(c) = map.creature_mut(creature)
        && let Some((_, t)) = c.threat.iter_mut().find(|(g, _)| *g == unit)
    {
        *t = (*t + delta).max(0.0);
    }
}

/// `creature` forgot its fight (died, evaded): nothing is held there now
/// (Thistle Tea's `Threat.wipe`).
pub fn wiped(map: &mut Map, creature: Guid) {
    map.spells.temp_threat.retain(|(c, _), _| *c != creature);
}

/// `creature` forgot `unit` (Thistle Tea's `Threat.remove`).
pub fn removed(map: &mut Map, creature: Guid, unit: Guid) {
    map.spells.temp_threat.remove(&(creature, unit));
}

/// `unit`'s temporary threat on `creature` is `amount` now.
pub fn set_temporary(map: &mut Map, creature: Guid, unit: Guid, amount: f32) {
    let tracked = map
        .creature(creature)
        .is_some_and(|c| c.threat.iter().any(|(g, _)| *g == unit));
    let previous = map
        .spells
        .temp_threat
        .get(&(creature, unit))
        .copied()
        .unwrap_or(0.0);
    if !tracked {
        // A fight forgotten (an evade, a death): nothing held there.
        map.spells.temp_threat.remove(&(creature, unit));
        return;
    }
    if previous == amount {
        return;
    }
    modify(map, creature, unit, -previous);
    modify(map, creature, unit, amount);
    if amount == 0.0 {
        map.spells.temp_threat.remove(&(creature, unit));
    } else {
        map.spells.temp_threat.insert((creature, unit), amount);
    }
}

/// `unit`'s threat auras changed: the creatures already fighting it take
/// their sum.
pub fn fade_changed(map: &mut Map, unit: Guid) {
    let total: f32 = auras::auras(map, unit).map_or(0.0, |a| {
        a.effects()
            .filter(|(_, x)| x.kind == AURA_MOD_TOTAL_THREAT)
            .map(|(h, x)| (x.amount * h.stacks.max(1) as i32) as f32)
            .sum()
    });
    let fighting: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.threat.iter().any(|(g, _)| *g == unit))
        .map(|(_, c)| c.guid)
        .collect();
    for c in fighting {
        set_temporary(map, c, unit, total);
    }
}

/// `taunter`'s attack-me effect on `creature`: its threat there matches
/// the creature's target's, for good (Thistle Tea's `Threat.taunt`).
pub fn matched(map: &mut Map, creature: Guid, taunter: Guid) {
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    let victim = c.combat.victim;
    if victim == taunter {
        return;
    }
    let Some(top) = c.threat.iter().find(|(g, _)| *g == victim).map(|(_, t)| *t) else {
        return;
    };
    match c.threat.iter_mut().find(|(g, _)| *g == taunter) {
        Some((_, t)) => *t = top.max(0.0),
        None => c.threat.push((taunter, top.max(0.0))),
    }
}

/// `taunter` taunted `creature`: it borrows what brings it level with the
/// creature's target, if it is below and borrows nothing yet.
pub fn taunted(map: &mut Map, creature: Guid, taunter: Guid) {
    let Some(c) = map.creature(creature) else {
        return;
    };
    if !c.alive() {
        return;
    }
    let victim = c.combat.victim;
    if victim == taunter {
        return;
    }
    let of = |g: Guid| c.threat.iter().find(|(t, _)| *t == g).map(|(_, t)| *t);
    let (Some(top), Some(mine)) = (of(victim), of(taunter)) else {
        return;
    };
    // A fight forgotten since leaves nothing borrowed.
    let borrowing = map
        .spells
        .temp_threat
        .get(&(creature, taunter))
        .is_some_and(|t| *t != 0.0 && mine >= *t);
    if mine < top && !borrowing {
        set_temporary(map, creature, taunter, top - mine);
    }
}

/// A taunt of `taunter`'s on `creature` ended: with none left, what it
/// borrowed goes back.
pub fn taunt_ended(map: &mut Map, creature: Guid, taunter: Guid) {
    let still = auras::auras(map, creature).is_some_and(|a| {
        a.holders
            .iter()
            .any(|h| h.caster == taunter && h.has(AURA_MOD_TAUNT))
    });
    if !still {
        set_temporary(map, creature, taunter, 0.0);
    }
}
