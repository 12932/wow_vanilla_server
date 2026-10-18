//! Alterac Valley's captured animals, as Thistle Tea's `CapturedFollower`
//! has them: a Frostwolf or an Alterac Ram, muzzled or collared, puts its
//! aura on the player (21827 or 21863, by its EventAI) and follows them
//! to their Stable Master. Every two seconds the aura looks at its animal:
//! dead or gone, more than fifty yards off, or the player dead, and the
//! aura goes, the animal a second after; else the animal follows again if
//! it has stopped (and isn't fighting). The aura ended otherwise (the
//! animal delivered, the aura cancelled, the player gone from the map)
//! sends the animal off two seconds after.

use std::time::Duration;

use wow_world_messages::Guid;

use crate::game::guid;
use crate::game::map::Map;

/// The capture auras, and the animal each is from.
const CAPTURES: [(u32, u32); 2] = [(21827, 10981), (21863, 10990)];
const CHECK: Duration = Duration::from_secs(2);
/// How far the animal may be from the player.
const REACH: f32 = 50.0;
/// How long after its aura goes the animal goes: lost, and otherwise.
const LOST_MS: u32 = 1000;
const RELEASED_MS: u32 = 2000;

/// How often a capture's aura looks at its animal, for a capture's.
pub fn period(spell: u32) -> Option<Duration> {
    CAPTURES.iter().any(|(s, _)| *s == spell).then_some(CHECK)
}

/// A capture's aura, from its own animal.
fn captured(spell: u32, caster: Guid) -> bool {
    CAPTURES
        .iter()
        .any(|(s, e)| *s == spell && guid::is_unit(caster) && guid::entry(caster) == *e)
}

/// The animal goes `ms` from now (sooner, if it was going sooner).
fn send_off(map: &mut Map, animal: Guid, ms: u32) {
    let at = map.now + Duration::from_millis(u64::from(ms));
    if map.creature(animal).is_none() {
        return;
    }
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, animal) {
        s.script_despawn = Some(match s.script_despawn {
            Some((t, r)) if t <= at => (t, r),
            _ => (at, None),
        });
    }
}

/// A capture aura's check on `player`: whether it was one.
pub fn tick(map: &mut Map, player: Guid, spell: u32, animal: Guid) -> bool {
    if !captured(spell, animal) {
        return false;
    }
    let alive = map.unit_view(player).is_some_and(|u| u.alive);
    let at = map.players.get(&player).map(|p| p.position());
    let kept = alive
        && at.is_some_and(|at| {
            map.creature(animal).is_some_and(|c| {
                c.alive() && crate::game::map::combat::dist(c.position, at) <= REACH
            })
        });
    if !kept {
        send_off(map, animal, LOST_MS);
        crate::game::map::auras::remove_spell(map, player, spell, Some(animal));
        return true;
    }
    let following = crate::game::map::event_ai::scripted(map, animal)
        .and_then(|s| s.follow)
        .is_some_and(|(whom, _, _)| whom == player);
    let busy = map
        .creature(animal)
        .is_some_and(|c| c.combat.attacking || c.evading);
    if !following && !busy {
        let step = crate::game::data::scripts::ScriptStep {
            command: 20,
            datalong: [15, 0, 0, 0],
            x: 3.0,
            ..Default::default()
        };
        crate::game::map::scripts::execute(map, &step, animal, player);
    }
    true
}

/// A capture's aura on `player` ended: its animal goes in two seconds,
/// unless it was lost and goes sooner.
pub fn ended(map: &mut Map, spell: u32, animal: Guid) {
    if captured(spell, animal) {
        send_off(map, animal, RELEASED_MS);
    }
}

/// `player` leaves the map (logging out, or for another): their captures
/// end.
pub fn leaving(map: &mut Map, player: Guid) {
    let held: Vec<(u32, Guid)> = crate::game::map::auras::auras(map, player)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| captured(h.spell, h.caster))
                .map(|h| (h.spell, h.caster))
                .collect()
        })
        .unwrap_or_default();
    for (spell, animal) in held {
        crate::game::map::auras::remove_spell(map, player, spell, Some(animal));
    }
}
