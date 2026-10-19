//! Duels, as mangos-zero runs them: the Duel spell plants a flag between
//! the two, the challenged accepts, a three-second countdown, and the
//! fight until one would fall (they kneel at 1 health instead), yields, or
//! stays more than 10 seconds beyond 75 yards of the flag (back within
//! 70).

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::combat::{self, dist};
use crate::game::map::gameobject::GameObject;

pub const FLAG_ENTRY: u32 = 21680;
/// The spell that kneels the loser.
pub const SPELL_GROVEL: u32 = 7267;
pub const COUNTDOWN: Duration = Duration::from_secs(3);
pub const BOUNDS_YARDS: f32 = 75.0;
/// How near the flag one out of bounds must come to be back in them.
pub const INBOUNDS_YARDS: f32 = 70.0;
pub const OUT_OF_BOUNDS_FOR: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Requested,
    Countdown { until: Instant },
    Fighting,
}

/// One side of a duel.
#[derive(Debug, Clone, Copy)]
pub struct Duel {
    pub opponent: Guid,
    pub flag: Guid,
    pub phase: Phase,
    pub initiator: bool,
    pub out_of_bounds_since: Option<Instant>,
}

/// Whether `a` and `b` are fighting a duel with each other.
pub fn dueling(map: &Map, a: Guid, b: Guid) -> bool {
    map.players
        .get(&a)
        .and_then(|p| p.duel)
        .is_some_and(|d| d.opponent == b && d.phase == Phase::Fighting)
}

/// Areas where duels may be fought (`AreaTable` flags).
pub const AREA_FLAG_DUELS: u32 = 0x40;
/// `SpellCastResult`s a challenge is refused with.
const FAIL_BAD_TARGETS: u8 = 0x0a;
const FAIL_NO_DUELING: u8 = 0x47;
const FAIL_TARGET_DUELING: u8 = 0x68;

/// Why `caster` can't challenge `target` with `s`, as Thistle Tea's duel
/// admission has it: only a player, on this map, not one who ignores the
/// challenger; neither already in a duel; both where duels are allowed
/// (an area whose flags are unknown allows them).
pub fn check(
    map: &Map,
    caster: Guid,
    s: &crate::game::data::spells::SpellTemplate,
    target: Option<Guid>,
) -> Option<u8> {
    if !s
        .effects
        .iter()
        .any(|e| e.effect == crate::game::map::spells::EFFECT_DUEL)
    {
        return None;
    }
    let (Some(a), Some(b)) = (
        map.players.get(&caster),
        target.and_then(|t| map.players.get(&t)),
    ) else {
        return Some(FAIL_BAD_TARGETS);
    };
    if a.guid == b.guid || map.realm.social.lists.lock().ignores(b.guid, a.guid) {
        return Some(FAIL_BAD_TARGETS);
    }
    if a.duel.is_some() || b.duel.is_some() {
        return Some(FAIL_TARGET_DUELING);
    }
    let allowed = |area: u32| {
        map.realm
            .data
            .area(area)
            .is_none_or(|i| i.flags & AREA_FLAG_DUELS != 0)
    };
    if !allowed(a.area) || !allowed(b.area) {
        return Some(FAIL_NO_DUELING);
    }
    None
}

/// The Duel spell's effect: `caster` challenges `target`.
pub fn challenge(map: &mut Map, caster: Guid, target: Guid) {
    let (Some(a), Some(b)) = (map.players.get(&caster), map.players.get(&target)) else {
        return;
    };
    if a.duel.is_some() || b.duel.is_some() || caster == target {
        return;
    }
    let (pa, pb) = (a.position(), b.position());
    let at = Vector3d {
        x: (pa.x + pb.x) / 2.0,
        y: (pa.y + pb.y) / 2.0,
        z: (pa.z + pb.z) / 2.0,
    };
    let data = map.realm.data.clone();
    let template = data
        .gameobject_templates
        .get(&FLAG_ENTRY)
        .cloned()
        .unwrap_or_else(|| GameObjectTemplate {
            entry: FLAG_ENTRY,
            kind: 16,
            display_id: 787,
            size: 1.0,
            name: "Duel Flag".into(),
            ..Default::default()
        });
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let spawn = GameObjectSpawn {
        guid: counter | 0x80_0000,
        entry: FLAG_ENTRY,
        map: map.key.map,
        x: at.x,
        y: at.y,
        z: at.z,
        state: 1,
        ..Default::default()
    };
    let mut flag = GameObject::from_spawn(&template, &spawn);
    flag.spawn_guid = None;
    flag.fields.set_guid(OBJECT_FIELD_CREATED_BY, caster);
    flag.fields.set_u32(GAMEOBJECT_LEVEL, 1);
    let flag = map.add_gameobject(flag);
    for (g, other, initiator) in [(caster, target, true), (target, caster, false)] {
        if let Some(p) = map.players.get_mut(&g) {
            p.duel = Some(Duel {
                opponent: other,
                flag,
                phase: Phase::Requested,
                initiator,
                out_of_bounds_since: None,
            });
            p.outbound.send(&SMSG_DUEL_REQUESTED {
                initiator: flag,
                target: caster,
            });
        }
    }
}

/// `CMSG_DUEL_ACCEPTED`: the challenged accepts, and the countdown starts.
pub fn accept(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(d) = map.players.get(&g).and_then(|p| p.duel) else {
        return;
    };
    if d.initiator || d.phase != Phase::Requested {
        return;
    }
    for p in [g, d.opponent] {
        if let Some(p) = map.players.get_mut(&p)
            && let Some(d) = p.duel.as_mut()
        {
            d.phase = Phase::Countdown {
                until: now + COUNTDOWN,
            };
            p.outbound.send(&SMSG_DUEL_COUNTDOWN { time: COUNTDOWN });
        }
    }
}

/// `CMSG_DUEL_CANCELLED`: before the fight it's called off; in it, the one
/// cancelling yields.
pub fn cancel(map: &mut Map, g: Guid) {
    let Some(d) = map.players.get(&g).and_then(|p| p.duel) else {
        return;
    };
    if d.phase == Phase::Fighting {
        yield_to(map, g);
    } else {
        end(map, g, None);
    }
}

/// `loser` yields or would fall: they kneel, and the other wins.
pub fn yield_to(map: &mut Map, loser: Guid) {
    let Some(d) = map.players.get(&loser).and_then(|p| p.duel) else {
        return;
    };
    // Both sides' fights end, their pets' too (mangos's `CombatStopWithPets`
    // as a duel is won or yielded).
    combat::stop_with_pets(map, d.opponent);
    combat::stop_with_pets(map, loser);
    crate::game::map::spells::cast_triggered(map, loser, SPELL_GROVEL, Some(loser));
    end(map, loser, Some(DuelWinnerReason::Won));
}

/// The duel ends: `None` interrupted, or with how the other side won.
fn end(map: &mut Map, loser: Guid, how: Option<DuelWinnerReason>) {
    let Some(d) = map.players.get(&loser).and_then(|p| p.duel) else {
        return;
    };
    let winner = d.opponent;
    for p in [loser, winner] {
        if let Some(p) = map.players.get_mut(&p) {
            p.duel = None;
            p.fields.set_guid(PLAYER_DUEL_ARBITER, Guid::zero());
            p.fields.set_u32(PLAYER_DUEL_TEAM, 0);
            p.outbound.send(&SMSG_DUEL_COMPLETE {
                ended_without_interruption: how.is_some(),
            });
        }
    }
    // Their pets stop fighting the other side.
    for (g, other) in [(loser, winner), (winner, loser)] {
        if let Some(pet) = crate::game::map::pets::pet_of(map, g) {
            let foes = [
                other,
                crate::game::map::pets::pet_of(map, other).unwrap_or_default(),
            ];
            if map
                .creature(pet)
                .is_some_and(|c| c.combat.attacking && foes.contains(&c.combat.victim))
            {
                combat::stop_attack(map, pet);
                if let Some(c) = map.creature_mut(pet) {
                    c.threat.retain(|(t, _)| !foes.contains(t));
                }
            }
        }
    }
    if let Some(reason) = how {
        let name = |g: Guid| {
            map.players
                .get(&g)
                .map(|p| p.name.to_string())
                .unwrap_or_default()
        };
        if let Some(frame) = crate::game::conn::encode(&SMSG_DUEL_WINNER {
            reason,
            opponent_name: name(loser),
            initiator_name: name(winner),
        }) {
            map.send_to_observers(winner, &frame, true);
        }
    }
    map.remove_gameobject(d.flag);
}

/// A blow in a duel that would fell its victim leaves them at 1 health,
/// and loses them the duel. Returns the damage to deal.
pub fn soften(map: &mut Map, attacker: Guid, victim: Guid, damage: u32) -> u32 {
    if !dueling(map, victim, attacker) {
        return damage;
    }
    let health = map
        .players
        .get(&victim)
        .map_or(0, |p| p.fields.get_u32(UNIT_FIELD_HEALTH));
    if damage < health {
        return damage;
    }
    if let Some(p) = map.players.get_mut(&victim) {
        p.fields.set_u32(UNIT_FIELD_HEALTH, 1);
    }
    yield_to(map, victim);
    0
}

/// Countdowns that run out start the fight; those out of bounds too long
/// flee.
pub fn update(map: &mut Map) {
    let now = map.now;
    let duels: Vec<(Guid, Duel)> = map
        .players
        .values()
        .filter_map(|p| p.duel.map(|d| (p.guid, d)))
        .collect();
    for (g, d) in duels {
        match d.phase {
            Phase::Countdown { until } if until <= now => {
                if let Some(p) = map.players.get_mut(&g)
                    && let Some(duel) = p.duel.as_mut()
                {
                    duel.phase = Phase::Fighting;
                    p.fields.set_guid(PLAYER_DUEL_ARBITER, d.flag);
                    p.fields
                        .set_u32(PLAYER_DUEL_TEAM, if d.initiator { 1 } else { 2 });
                }
            }
            Phase::Fighting => {
                let Some(flag) = map.gameobject(d.flag).map(|f| f.position) else {
                    end(map, g, None);
                    continue;
                };
                let Some(pos) = map.players.get(&g).map(|p| p.position()) else {
                    continue;
                };
                // Out past the bounds, back in only well inside them
                // (Thistle Tea's outbound and inbound radii, as mangos has
                // two).
                let distance = dist(pos, flag);
                let Some(p) = map.players.get_mut(&g) else {
                    continue;
                };
                let Some(duel) = p.duel.as_mut() else {
                    continue;
                };
                match duel.out_of_bounds_since {
                    None if distance > BOUNDS_YARDS => {
                        duel.out_of_bounds_since = Some(now);
                        p.outbound.send(&SMSG_DUEL_OUTOFBOUNDS {});
                    }
                    Some(_) if distance <= INBOUNDS_YARDS => {
                        duel.out_of_bounds_since = None;
                        p.outbound.send(&SMSG_DUEL_INBOUNDS {});
                    }
                    Some(since) if now.duration_since(since) >= OUT_OF_BOUNDS_FOR => {
                        end(map, g, Some(DuelWinnerReason::Fled));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// A player leaving the map (logout, teleport) flees their duel; dying
/// to something else interrupts it.
pub fn left(map: &mut Map, g: Guid, flees: bool) {
    let fighting = map
        .players
        .get(&g)
        .and_then(|p| p.duel)
        .map(|d| d.phase == Phase::Fighting);
    match fighting {
        Some(true) if flees => end(map, g, Some(DuelWinnerReason::Fled)),
        // Interrupted by another's killing blow: both fights end, as a win
        // ends them (mangos's `DealDamage`).
        Some(true) => {
            if let Some(other) = map.players.get(&g).and_then(|p| p.duel).map(|d| d.opponent) {
                combat::stop_with_pets(map, other);
            }
            combat::stop_with_pets(map, g);
            end(map, g, None);
        }
        Some(false) => end(map, g, None),
        None => {}
    }
}
