//! Creature AI: noticing a hostile player, threat, chasing on the navmesh,
//! leashing home, calling for help, death, corpse decay and respawn.

use std::sync::Arc;
use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::items::Body;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::{Creature, Life, Motion};
use crate::game::map::death::Death;
use crate::game::map::player::{PLAYER_FLAGS_GM, UNIT_FLAG_IN_COMBAT};

/// How often an idle hostile creature looks for players.
const AGGRO_SCAN_EVERY: Duration = Duration::from_millis(500);
/// How far a creature follows from home before it gives up.
const DEFAULT_LEASH: f32 = 60.0;
/// A chasing creature paths again when its victim has moved this far.
const REPATH_YARDS: f32 = 1.5;
/// How long a corpse lies before it goes.
const CORPSE_DECAY: Duration = Duration::from_secs(60);
const ELITE_CORPSE_DECAY: Duration = Duration::from_secs(300);
/// The shortest wait before a creature respawns.
const MIN_RESPAWN: Duration = Duration::from_secs(5);
/// `UNIT_DYNAMIC_FLAGS`: the creature is tapped by someone else.
pub const DYNFLAG_TAPPED: u32 = 0x4;

pub const SMSG_MONSTER_MOVE: u16 = 0x0DD;

/// A short `SMSG_MONSTER_MOVE`: the creature stops where it is.
pub fn stop_packet(g: Guid, at: Vector3d, spline_id: u32) -> Arc<[u8]> {
    let mut b = Body::default();
    crate::game::update::write_packed_guid(&mut b.0, g);
    b.f32(at.x).f32(at.y).f32(at.z).u32(spline_id).u8(1);
    b.frame(SMSG_MONSTER_MOVE)
}

/// The radius at which a creature notices a player (vmangos's
/// `GetAttackDistance`): about 20 yards at an even level, a yard less for
/// each level the player is above it, at least 5.
pub fn aggro_radius(detection: f32, creature_level: u32, player_level: u32) -> f32 {
    let base = if detection > 0.0 { detection } else { 20.0 };
    let diff = (player_level as i32 - creature_level as i32).max(-25);
    (base - diff as f32).clamp(5.0, 45.0)
}

/// `creature` was attacked by `attacker`: it fights back.
pub fn attacked(map: &mut Map, creature: Guid, attacker: Guid) {
    let now = map.now;
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    if !c.alive() || c.evading {
        return;
    }
    if c.tapped_by.is_none() && guid::is_player(attacker) {
        c.tapped_by = Some(attacker);
    }
    if c.threat.iter().all(|(g, _)| *g != attacker) {
        c.threat.push((attacker, 0.0));
    }
    if c.combat.attacking {
        return;
    }
    c.combat_start = Some(now);
    c.combat.victim = attacker;
    c.combat.attacking = true;
    c.combat.next_swing.get_or_insert(now);
    c.fields.set_guid(UNIT_FIELD_TARGET, attacker);
    c.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, true);
    if let Some(frame) = crate::game::conn::encode(&wow_world_messages::vanilla::SMSG_ATTACKSTART {
        attacker: creature,
        victim: attacker,
    }) {
        map.send_to_observers(creature, &frame, false);
        if let Some(p) = map.players.get(&attacker)
            && !p.known.contains(&creature)
        {
            p.outbound.send_frame(frame);
        }
    }
    call_for_help(map, creature, attacker);
}

/// Damage from `from` raises its threat on `creature`.
pub fn add_threat(map: &mut Map, creature: Guid, from: Guid, amount: f32) {
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    match c.threat.iter_mut().find(|(g, _)| *g == from) {
        Some((_, t)) => *t += amount,
        None => c.threat.push((from, amount)),
    }
}

/// Creatures forget `g` (feigning death, vanishing): its threat goes, and
/// one fighting it turns to the next on its list.
pub fn forget(map: &mut Map, g: Guid) {
    let fighting: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.threat.iter().any(|(t, _)| *t == g))
        .map(|(_, c)| c.guid)
        .collect();
    for c in fighting {
        let next = {
            let Some(cr) = map.creature_mut(c) else {
                continue;
            };
            cr.threat.retain(|(t, _)| *t != g);
            cr.tapped_by = cr.tapped_by.filter(|t| *t != g);
            if cr.combat.victim != g {
                continue;
            }
            cr.threat
                .iter()
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(t, _)| *t)
        };
        combat::stop_attack(map, c);
        match next {
            Some(t) => {
                if let Some(cr) = map.creature_mut(c) {
                    cr.combat.attacking = false;
                }
                attacked(map, c, t);
                if let Some(cr) = map.creature_mut(c) {
                    cr.combat.victim = t;
                    cr.combat.attacking = true;
                    cr.fields.set_guid(UNIT_FIELD_TARGET, t);
                }
            }
            None => evade(map, c),
        }
    }
}

/// A taunt: `by` goes to the top of `creature`'s threat, and is its victim
/// while the taunt lasts.
pub fn taunted(map: &mut Map, creature: Guid, by: Guid) {
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    let top = c.threat.iter().map(|(_, t)| *t).fold(0.0f32, f32::max);
    match c.threat.iter_mut().find(|(g, _)| *g == by) {
        Some((_, t)) => *t = t.max(top),
        None => c.threat.push((by, top)),
    }
    c.taunted_by = Some(by);
    c.combat.victim = by;
    c.fields.set_guid(UNIT_FIELD_TARGET, by);
    attacked(map, creature, by);
}

pub fn taunt_ended(map: &mut Map, creature: Guid, by: Guid) {
    if let Some(c) = map.creature_mut(creature)
        && c.taunted_by == Some(by)
    {
        c.taunted_by = None;
    }
}

/// Creatures of the same faction near `creature` join its fight.
fn call_for_help(map: &mut Map, creature: Guid, enemy: Guid) {
    let Some(c) = map.creature(creature) else {
        return;
    };
    let template = map.realm.data.creature_template(c.entry);
    let radius = template
        .map(|t| t.call_for_help_range)
        .filter(|r| *r > 0.0)
        .unwrap_or(0.0);
    if radius <= 0.0 {
        return;
    }
    let (pos, faction) = (c.position, c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE));
    let helpers: Vec<Guid> = map
        .grid
        .around(pos.x, pos.y, radius)
        .filter(|g| guid::is_unit(*g) && *g != creature)
        .filter(|g| {
            map.creature(*g).is_some_and(|h| {
                h.alive()
                    && !h.combat.attacking
                    && h.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE) == faction
                    && dist(h.position, pos) <= radius
            })
        })
        .collect();
    for h in helpers {
        attacked(map, h, enemy);
    }
}

/// A creature dies.
pub fn die(map: &mut Map, creature: Guid, tapper: Guid) {
    crate::game::map::auras::clear_on_death(map, creature);
    let now = map.now;
    let data = map.realm.data.clone();
    let spline_id = map.next_spline_id;
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    let template = data.creature_template(c.entry);
    let elite = template.is_some_and(|t| t.rank == 1 || t.rank == 2);
    let respawn = c
        .spawn_guid
        .and_then(|g| {
            data.creature_spawns
                .values()
                .flatten()
                .find(|s| s.guid == g)
        })
        .map(|s| {
            let (lo, hi) = (s.respawn_min, s.respawn_max.max(s.respawn_min));
            let secs = if hi > lo {
                rand::rng().random_range(lo..=hi)
            } else {
                lo
            };
            Duration::from_secs(u64::from(secs))
        })
        .unwrap_or(Duration::from_secs(300))
        .max(MIN_RESPAWN);
    let decay = if elite {
        ELITE_CORPSE_DECAY
    } else {
        CORPSE_DECAY
    };
    c.life = Life::Dead {
        corpse_until: now + decay.min(respawn),
        respawn_at: now + respawn.max(decay.min(respawn)),
    };
    let moving = matches!(c.motion, Motion::Moving(_));
    c.motion = Motion::Idle { until: None };
    c.threat.clear();
    c.combat = Default::default();
    c.evading = false;
    c.fields.set_u32(UNIT_FIELD_HEALTH, 0);
    c.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
    c.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, false);
    c.tapped_by = Some(tapper);
    let pos = c.position;
    map.dead_creatures.push(creature);
    if moving {
        map.next_spline_id = map.next_spline_id.wrapping_add(1);
        let frame = stop_packet(creature, pos, spline_id);
        map.send_to_observers(creature, &frame, false);
    }
    crate::game::map::loot::creature_died(map, creature, tapper);
}

/// Corpses that decay and creatures that respawn.
pub fn update_dead(map: &mut Map) {
    if map.dead_creatures.is_empty() {
        return;
    }
    let now = map.now;
    let dead = std::mem::take(&mut map.dead_creatures);
    let mut still = Vec::with_capacity(dead.len());
    for g in dead {
        let Some(c) = map.creature_mut(g) else {
            continue;
        };
        let Life::Dead {
            corpse_until,
            respawn_at,
        } = c.life
        else {
            continue;
        };
        if respawn_at <= now {
            respawn(map, g);
            continue;
        }
        if corpse_until <= now && !c.despawned {
            c.despawned = true;
            despawn_from_view(map, g);
        }
        still.push(g);
    }
    map.dead_creatures.extend(still);
}

/// Take a creature out of every client's view and out of the grid; it
/// stays in the map until it respawns.
fn despawn_from_view(map: &mut Map, g: Guid) {
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let cell = c.cell;
    let observers: Vec<Guid> = c.observers.drain().collect();
    map.grid.remove(g, cell);
    for o in observers {
        if let Some(p) = map.players.get_mut(&o) {
            p.known.remove(&g);
            map.forget.entry(o).or_default().push(g);
        }
    }
}

/// Bring a dead creature back at its spawn, as new.
fn respawn(map: &mut Map, g: Guid) {
    let Some(old) = map.creature(g) else {
        return;
    };
    let spawn_guid = old.spawn_guid;
    let despawned = old.despawned;
    let data = map.realm.data.clone();
    let spawn = spawn_guid.and_then(|sg| {
        data.creature_spawns
            .get(&map.key.map)
            .and_then(|v| v.iter().find(|s| s.guid == sg))
    });
    if !despawned {
        despawn_from_view(map, g);
    }
    if let Some(c) = map
        .creature_by_guid
        .remove(&g)
        .and_then(|k| map.creatures.try_remove(k))
    {
        drop(c);
    }
    let Some(spawn) = spawn else {
        return;
    };
    if let Some(c) = Creature::from_spawn(&data, spawn, &mut rand::rng()) {
        map.add_creature(c);
    }
}

/// Run the creatures some player sees, or that are fighting.
pub fn update_creatures(map: &mut Map) {
    let now = map.now;
    let keys: Vec<usize> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.alive() && (!c.observers.is_empty() || c.combat.attacking || c.evading))
        .map(|(k, _)| k)
        .collect();
    for key in keys {
        let Some(c) = map.creatures.get(key) else {
            continue;
        };
        let g = c.guid;
        if c.evading {
            evade_step(map, g);
        } else if c.combat.attacking {
            fight_step(map, g);
        } else {
            if c.hostile_to_players && c.aggro_check_at <= now && !c.observers.is_empty() {
                aggro_scan(map, g);
            }
            if !map.creature(g).is_some_and(|c| c.combat.attacking) {
                crate::game::map::world::step_creature_key(map, key);
            }
        }
    }
}

/// Look for a player to attack.
fn aggro_scan(map: &mut Map, g: Guid) {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.aggro_check_at = now + AGGRO_SCAN_EVERY;
    let (pos, level) = (c.position, c.fields.get_u32(UNIT_FIELD_LEVEL));
    let detection = data
        .creature_template(c.entry)
        .map(|t| t.detection_range)
        .unwrap_or(20.0);
    let candidates: Vec<Guid> = map
        .players_near(pos.x, pos.y, 45.0)
        .into_iter()
        .filter(|p| {
            map.players.get(p).is_some_and(|pl| {
                pl.death == Death::Alive
                    && !pl.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_GM)
                    && !pl.awaiting_worldport
                    && dist(pl.position(), pos)
                        <= aggro_radius(detection, level, u32::from(pl.level()))
            })
        })
        .collect();
    let target = candidates
        .into_iter()
        .filter(|p| combat::hostile(map, g, *p))
        .min_by(|a, b| {
            let da = map
                .players
                .get(a)
                .map(|p| dist(p.position(), pos))
                .unwrap_or(f32::MAX);
            let db = map
                .players
                .get(b)
                .map(|p| dist(p.position(), pos))
                .unwrap_or(f32::MAX);
            da.total_cmp(&db)
        });
    let Some(target) = target else {
        return;
    };
    let line_clear = match (map.nav.as_mut(), map.players.get(&target)) {
        (Some(nav), Some(p)) => {
            let eye = |v: Vector3d| Vector3d { z: v.z + 1.5, ..v };
            nav.line_of_sight(eye(pos), eye(p.position()))
        }
        _ => true,
    };
    if line_clear {
        attacked(map, g, target);
    }
}

/// The highest threat still worth fighting, if any.
fn pick_victim(map: &Map, g: Guid) -> Option<Guid> {
    let c = map.creature(g)?;
    let home = c.home;
    let leash = leash_of(map, c);
    c.threat
        .iter()
        .filter(|(t, _)| {
            map.unit_view(*t).is_some_and(|u| {
                u.alive && dist(u.position, home) <= leash && combat::can_attack(map, g, *t)
            })
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(t, _)| *t)
}

fn leash_of(map: &Map, c: &Creature) -> f32 {
    map.realm
        .data
        .creature_template(c.entry)
        .map(|t| t.leash_range)
        .filter(|l| *l > 0.0)
        .unwrap_or(DEFAULT_LEASH)
}

fn fight_step(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(c) = map.creature(g) else {
        return;
    };
    let leash = leash_of(map, c);
    let (home, pos) = (c.home, c.position);
    let current = c.combat.victim;
    let victim = if map
        .unit_view(current)
        .is_some_and(|u| u.alive && dist(u.position, home) <= leash)
        && dist(pos, home) <= leash
        && combat::can_attack(map, g, current)
    {
        Some(current)
    } else {
        pick_victim(map, g)
    };
    let Some(victim) = victim.filter(|_| dist(pos, home) <= leash) else {
        evade(map, g);
        return;
    };
    if victim != current
        && let Some(c) = map.creature_mut(g)
    {
        c.combat.victim = victim;
        c.fields.set_guid(UNIT_FIELD_TARGET, victim);
    }
    advance_motion(map, g);
    // Stunned, or casting: it stands where it is.
    crate::game::map::spells::creature_cast(map, g, victim);
    let busy = map.creature(g).is_some_and(|c| c.caster.casting.is_some())
        || crate::game::map::auras::held(map, g, false);
    if busy {
        stop_moving(map, g);
        return;
    }
    let (Some(me), Some(them)) = (map.unit_view(g), map.unit_view(victim)) else {
        return;
    };
    if combat::in_melee_range(&me, &them) {
        let spline_id = map.next_spline_id;
        let Some(c) = map.creature_mut(g) else {
            return;
        };
        c.orientation = (them.position.y - me.position.y).atan2(them.position.x - me.position.x);
        if matches!(c.motion, Motion::Moving(_)) {
            c.motion = Motion::Idle { until: None };
            map.next_spline_id = map.next_spline_id.wrapping_add(1);
            let frame = stop_packet(g, me.position, spline_id);
            map.send_to_observers(g, &frame, false);
        }
        combat::try_swing(map, g);
        return;
    }
    // Chase: path again if the victim has moved since the last one.
    let Some(c) = map.creature(g) else {
        return;
    };
    let stale = c
        .chase_target_at
        .is_none_or(|at| dist(at, them.position) > REPATH_YARDS)
        || !matches!(c.motion, Motion::Moving(_));
    if !stale {
        return;
    }
    let reach = combat::melee_range(&me, &them) * 0.8;
    let to_me = Vector3d {
        x: me.position.x - them.position.x,
        y: me.position.y - them.position.y,
        z: 0.0,
    };
    let len = (to_me.x * to_me.x + to_me.y * to_me.y).sqrt().max(0.01);
    let goal = Vector3d {
        x: them.position.x + to_me.x / len * reach.min(len),
        y: them.position.y + to_me.y / len * reach.min(len),
        z: them.position.z,
    };
    let path = match map.nav.as_mut() {
        Some(nav) => nav
            .path(me.position, goal)
            .unwrap_or_else(|| vec![me.position, goal]),
        None => vec![me.position, goal],
    };
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.chase_target_at = Some(them.position);
    if let Some(frame) = c.walk(path, true, now, spline_id) {
        map.send_to_observers(g, &frame, false);
    }
}

/// Give up the fight and run home.
fn evade(map: &mut Map, g: Guid) {
    let now = map.now;
    let spline_id = map.next_spline_id;
    let Some(c) = map.creature(g) else {
        return;
    };
    let (from, home) = (c.position, c.home);
    let path = match map.nav.as_mut() {
        Some(nav) => nav.path(from, home).unwrap_or_else(|| vec![from, home]),
        None => vec![from, home],
    };
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let attackers: Vec<Guid> = map
        .creature(g)
        .map(|c| c.threat.iter().map(|(t, _)| *t).collect())
        .unwrap_or_default();
    combat::stop_attack(map, g);
    crate::game::map::spells::interrupt(map, g);
    let held: Vec<u32> = map
        .creature(g)
        .map(|c| c.auras.holders.iter().map(|a| a.spell).collect())
        .unwrap_or_default();
    for spell in held {
        crate::game::map::auras::remove_spell(map, g, spell, None);
    }
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.evading = true;
    c.threat.clear();
    c.combat = Default::default();
    c.caster = Default::default();
    c.tapped_by = None;
    c.chase_target_at = None;
    c.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
    c.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, false);
    c.fields.set_flag(UNIT_DYNAMIC_FLAGS, DYNFLAG_TAPPED, false);
    let frame = c.walk(path, true, now, spline_id);
    if frame.is_none() {
        c.evading = false;
    }
    if let Some(frame) = frame {
        map.send_to_observers(g, &frame, false);
    }
    for a in attackers {
        if map.combat(a).is_some_and(|c| c.attacking && c.victim == g) {
            combat::stop_attack(map, a);
        }
        if let Some(c) = map.combat_mut(a) {
            c.attackers.remove(&g);
        }
    }
}

/// An evading creature walks home, and is whole again when it gets there.
fn evade_step(map: &mut Map, g: Guid) {
    let now = map.now;
    advance_motion(map, g);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if matches!(c.motion, Motion::Idle { .. }) {
        c.evading = false;
        let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        c.fields.set_u32(UNIT_FIELD_HEALTH, max);
        c.orientation = c.home_orientation;
        c.motion = Motion::Idle {
            until: Some(now + Duration::from_secs(2)),
        };
    }
}

/// Bring a moving creature's position up to now, and its grid cell and
/// visibility with it.
pub fn advance_motion(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let Motion::Moving(w) = &c.motion else {
        return;
    };
    let done = w.done(now);
    let (pos, o) = if done {
        (w.destination(), c.orientation)
    } else {
        w.position_at(now)
    };
    c.position = pos;
    c.orientation = o;
    if done {
        c.motion = Motion::Idle { until: None };
    }
    let (anchor, old_cell) = (c.vis_anchor, c.cell);
    let new_cell = crate::game::map::grid::cell_of(pos.x, pos.y);
    c.cell = new_cell;
    map.grid.relocate(g, old_cell, new_cell);
    crate::game::map::visibility::moved(map, g, anchor, pos);
}

/// Whether a creature's faction is hostile to the players' factions, so it
/// looks for them.
pub fn hostile_to_players(
    data: &crate::game::data::GameData,
    faction: u32,
    civilian: bool,
) -> bool {
    if civilian {
        return false;
    }
    let Some(f) = data.faction_templates.get(&faction) else {
        return false;
    };
    // 1: Alliance players' group, 2: Horde's ... the player masks.
    f.enemy_mask & 0x7 != 0
        || f.enemies
            .iter()
            .any(|e| matches!(*e, 469 | 67 | 1 | 2 | 3 | 4 | 5 | 6 | 8 | 115 | 116))
}

/// A creature on the move stops where it is.
fn stop_moving(map: &mut Map, g: Guid) {
    let spline_id = map.next_spline_id;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if !matches!(c.motion, Motion::Moving(_)) {
        return;
    }
    c.motion = Motion::Idle { until: None };
    let at = c.position;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let frame = stop_packet(g, at, spline_id);
    map.send_to_observers(g, &frame, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_aggro_radius_shrinks_as_the_player_outlevels() {
        assert_eq!(aggro_radius(20.0, 10, 10), 20.0);
        assert_eq!(aggro_radius(20.0, 10, 15), 15.0);
        assert_eq!(aggro_radius(20.0, 10, 40), 5.0);
        assert_eq!(aggro_radius(20.0, 60, 1), 45.0);
    }

    #[test]
    fn a_stop_packet_is_short() {
        let f = stop_packet(
            guid::unit(1, 1),
            Vector3d {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            4,
        );
        // Header, packed guid (4 bytes here), position, id, type.
        assert_eq!(f.len(), 4 + 5 + 12 + 4 + 1);
    }
}
