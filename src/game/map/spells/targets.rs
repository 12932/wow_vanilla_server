//! Whom a spell's effects land on: its implicit targets (as Thistle Tea
//! names vmangos's), chains that jump from one to the next, areas at the
//! caster, the destination or the channel, cones, parties and raids.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::spells::SpellTemplate;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::{self, dist};

use super::Targets;
use super::mods::{self, Mods};

/// How far a chain jumps.
pub const CHAIN_JUMP_YARDS: f32 = 10.0;

/// An effect's implicit target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Implicit {
    None,
    Caster,
    Enemy,
    Ally,
    AnyUnit,
    Pet,
    CasterMaster,
    ChainHeal,
    PartyMember,
    PartyAroundCaster,
    PartyAroundTarget,
    RaidAroundCaster,
    RaidAndClass,
    AoeEnemyAtCaster,
    AoeEnemyAtDest,
    AoeEnemyInCone,
    AoeEnemyAtChannel,
    AoeAllyAtSource,
    AoeAllyAtDest,
    ScriptUnits,
    CreatureNearCaster,
    GameObject,
    /// A place: the destination, near the caster, the caster's own.
    Location,
    EnemyLocation,
    Other(u32),
}

pub fn implicit(t: u32) -> Implicit {
    use Implicit::*;
    match t {
        0 => None,
        1 => Caster,
        5 => Pet,
        6 => Enemy,
        7 | 8 | 60 => ScriptUnits,
        9 | 17 | 18 | 22 | 26 | 32 | 39 | 41..=44 | 46..=50 | 55 => Location,
        15 => AoeEnemyAtCaster,
        16 => AoeEnemyAtDest,
        20 | 33 | 34 => PartyAroundCaster,
        21 | 57 => Ally,
        23 | 40 | 51 | 52 => GameObject,
        24 | 54 => AoeEnemyInCone,
        25 => AnyUnit,
        27 => CasterMaster,
        28 => AoeEnemyAtChannel,
        30 => AoeAllyAtSource,
        31 => AoeAllyAtDest,
        35 => PartyMember,
        37 => PartyAroundTarget,
        38 => CreatureNearCaster,
        45 => ChainHeal,
        53 | 63 => EnemyLocation,
        56 => RaidAroundCaster,
        61 => RaidAndClass,
        other => Other(other),
    }
}

/// The targets wvs resolves.
pub fn supported(t: u32) -> bool {
    !matches!(implicit(t), Implicit::Other(_)) || (2..=4).contains(&t)
}

/// Whether a target wants a unit picked by the client.
pub fn wants_unit(t: u32) -> bool {
    matches!(
        implicit(t),
        Implicit::Enemy
            | Implicit::Ally
            | Implicit::AnyUnit
            | Implicit::ChainHeal
            | Implicit::PartyMember
    ) || t == 53
}

/// An effect's radius, with the caster's modifiers.
pub fn radius(map: &Map, s: &SpellTemplate, index: usize, m: &Mods) -> f32 {
    let r = map.realm.data.spells.radius(s.effects[index].radius_index);
    m.apply(mods::OP_RADIUS, r).max(0.0)
}

fn members(map: &Map, g: Guid, raid_wide: bool) -> Vec<Guid> {
    let groups = map.realm.social.groups.lock();
    match groups.group_of(g) {
        Some(gr) => {
            let sub = gr.members.iter().find(|m| m.guid == g).map(|m| m.subgroup);
            gr.members
                .iter()
                .filter(|m| raid_wide || !gr.raid || Some(m.subgroup) == sub)
                .map(|m| m.guid)
                .collect()
        }
        None => vec![g],
    }
}

fn alive(map: &Map, g: Guid) -> bool {
    map.unit_view(g).is_some_and(|u| u.alive)
}

/// Units around `at` within `radius`, that `pick` keeps.
fn units_around(
    map: &Map,
    at: Vector3d,
    radius: f32,
    mut pick: impl FnMut(Guid) -> bool,
) -> Vec<Guid> {
    let mut v: Vec<(Guid, f32)> = map
        .grid
        .around(at.x, at.y, radius)
        .filter(|g| guid::is_player(*g) || guid::is_unit(*g))
        .filter_map(|g| map.position_of(g).map(|p| (g, dist(p, at))))
        .filter(|(_, d)| *d <= radius)
        .filter(|(g, _)| pick(*g))
        .collect();
    v.sort_by(|a, b| a.1.total_cmp(&b.1));
    v.into_iter().map(|(g, _)| g).collect()
}

/// Whether `g` is in front of `caster`, within `degrees`.
fn in_cone(map: &Map, caster: Guid, g: Guid, degrees: i32) -> bool {
    let (Some(me), Some(p)) = (map.unit_view(caster), map.position_of(g)) else {
        return false;
    };
    let a = (p.y - me.position.y).atan2(p.x - me.position.x) - me.orientation;
    let a = a.sin().atan2(a.cos()).abs();
    a <= degrees as f32 * std::f32::consts::PI / 360.0
}

/// The point an effect's area is at.
fn center(map: &Map, caster: Guid, t: &Targets, at_dest: bool) -> Vector3d {
    let caster_at = map.position_of(caster).unwrap_or(Vector3d {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    if !at_dest {
        return caster_at;
    }
    t.dest
        .or_else(|| t.unit.and_then(|u| map.position_of(u)))
        .unwrap_or(caster_at)
}

/// The units effect `index` of `s` lands on.
pub fn effect_targets(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    t: &Targets,
    m: &Mods,
) -> Vec<Guid> {
    use Implicit::*;
    let e = &s.effects[index];
    let (a, b) = (implicit(e.target_a), implicit(e.target_b));
    let r = radius(map, s, index, m).max(if e.radius_index == 0 { 0.0 } else { 1.0 });
    let enemy = |g: Guid| alive(map, g) && combat::can_attack(map, caster, g);
    let ally = |g: Guid| {
        alive(map, g)
            && (g == caster
                || (!combat::can_attack(map, caster, g) && !combat::hostile(map, caster, g)))
    };
    let mut out = match (a, b) {
        // An area named by the second target.
        (_, AoeEnemyAtCaster) | (AoeEnemyAtCaster, _) => {
            units_around(map, center(map, caster, t, false), r, |g| {
                g != caster && enemy(g)
            })
        }
        (_, AoeEnemyAtDest)
        | (AoeEnemyAtDest, _)
        | (_, AoeEnemyAtChannel)
        | (AoeEnemyAtChannel, _) => units_around(map, center(map, caster, t, true), r, |g| {
            g != caster && enemy(g)
        }),
        (_, AoeEnemyInCone) | (AoeEnemyInCone, _) => {
            let degrees = map
                .realm
                .data
                .spells
                .cones
                .get(&s.id)
                .copied()
                .unwrap_or(60);
            units_around(map, center(map, caster, t, false), r, |g| {
                g != caster && enemy(g) && in_cone(map, caster, g, degrees)
            })
        }
        (_, AoeAllyAtSource) | (AoeAllyAtSource, _) => {
            units_around(map, center(map, caster, t, false), r, ally)
        }
        (_, AoeAllyAtDest) | (AoeAllyAtDest, _) => {
            units_around(map, center(map, caster, t, true), r, ally)
        }
        (PartyAroundCaster, _) | (_, PartyAroundCaster) => {
            let at = center(map, caster, t, e.target_a == 34);
            let party = members(map, caster, false);
            party
                .into_iter()
                .filter(|g| alive(map, *g))
                .filter(|g| {
                    r <= 0.0
                        || map
                            .position_of(*g)
                            .is_some_and(|p| dist(p, at) <= r.max(30.0))
                })
                .collect()
        }
        (RaidAroundCaster, _) => {
            let at = center(map, caster, t, false);
            members(map, caster, true)
                .into_iter()
                .filter(|g| alive(map, *g))
                .filter(|g| {
                    map.position_of(*g)
                        .is_some_and(|p| dist(p, at) <= r.max(30.0))
                })
                .collect()
        }
        (PartyAroundTarget, _) => {
            let target = t.unit.unwrap_or(caster);
            let at = map
                .position_of(target)
                .unwrap_or(center(map, caster, t, false));
            members(map, target, false)
                .into_iter()
                .filter(|g| alive(map, *g))
                .filter(|g| {
                    map.position_of(*g)
                        .is_some_and(|p| dist(p, at) <= r.max(30.0))
                })
                .collect()
        }
        (RaidAndClass, _) => {
            let target = t.unit.unwrap_or(caster);
            let class = map.players.get(&target).map(|p| p.row.class);
            let at = map
                .position_of(caster)
                .unwrap_or(center(map, caster, t, false));
            members(map, target, true)
                .into_iter()
                .filter(|g| alive(map, *g))
                .filter(|g| map.players.get(g).map(|p| p.row.class) == class)
                .filter(|g| {
                    map.position_of(*g)
                        .is_some_and(|p| dist(p, at) <= r.max(100.0))
                })
                .collect()
        }
        (PartyMember, _) => {
            let target = t.unit.unwrap_or(caster);
            if members(map, caster, true).contains(&target) {
                vec![target]
            } else {
                vec![caster]
            }
        }
        (Caster, _) | (None, _) => vec![caster],
        (Pet, _) => map
            .players
            .get(&caster)
            .map(|p| p.fields.get_guid(crate::game::fields::UNIT_FIELD_SUMMON))
            .filter(|g| !g.is_zero())
            .or_else(|| {
                map.creatures
                    .iter()
                    .find(|(_, c)| c.owner == Some(caster))
                    .map(|(_, c)| c.guid)
            })
            .into_iter()
            .collect(),
        (CasterMaster, _) => map
            .creature(caster)
            .and_then(|c| c.owner)
            .into_iter()
            .collect(),
        (ChainHeal, _) => {
            let first = t.unit.unwrap_or(caster);
            chain(map, caster, s, index, first, m, true)
        }
        (Enemy, _) | (AnyUnit, _) | (Ally, _) => {
            let first = t
                .unit
                .unwrap_or(if a == Ally { caster } else { Guid::zero() });
            if first.is_zero() {
                Vec::new()
            } else if e.chain_target > 1 {
                chain(map, caster, s, index, first, m, a == Ally)
            } else {
                vec![first]
            }
        }
        (EnemyLocation, _) => t.unit.into_iter().collect(),
        (ScriptUnits, _) | (CreatureNearCaster, _) => script_units(map, caster, s, t, r),
        (GameObject, _) | (Location, _) => Vec::new(),
        (Other(_), _) => t.unit.into_iter().collect(),
    };
    if s.max_affected_targets > 0 && out.len() > s.max_affected_targets as usize {
        out.truncate(s.max_affected_targets as usize);
    }
    out
}

/// A chain: the first target, then the nearest (or, for heals, the most
/// hurt) within a jump of the last, up to the effect's count.
fn chain(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    first: Guid,
    m: &Mods,
    heal: bool,
) -> Vec<Guid> {
    let count = m
        .apply(mods::OP_JUMP_TARGETS, s.effects[index].chain_target as f32)
        .max(1.0) as usize;
    let mut out = vec![first];
    let mut last = first;
    while out.len() < count {
        let Some(at) = map.position_of(last) else {
            break;
        };
        let next = units_around(map, at, CHAIN_JUMP_YARDS, |g| {
            !out.contains(&g)
                && alive(map, g)
                && if heal {
                    !combat::can_attack(map, caster, g) && !combat::hostile(map, caster, g)
                } else {
                    g != caster && combat::can_attack(map, caster, g)
                }
        });
        let pick = if heal {
            next.into_iter().min_by_key(|g| {
                map.unit_fields(*g).map_or(u32::MAX, |f| {
                    let max = f.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH).max(1);
                    f.get_u32(crate::game::fields::UNIT_FIELD_HEALTH) * 1000 / max
                })
            })
        } else {
            next.into_iter().next()
        };
        let Some(g) = pick else {
            break;
        };
        out.push(g);
        last = g;
    }
    out
}

/// The units a scripted spell aims at: the creatures `spell_script_target`
/// names, near the caster (or the target the client picked).
fn script_units(map: &Map, caster: Guid, s: &SpellTemplate, t: &Targets, radius: f32) -> Vec<Guid> {
    let entries: Vec<u32> = map
        .realm
        .data
        .spells
        .script_targets
        .get(&s.id)
        .map(|v| {
            v.iter()
                .filter(|(k, _)| *k == 1 || *k == 2)
                .map(|(_, e)| *e)
                .collect()
        })
        .unwrap_or_default();
    if entries.is_empty() {
        return t.unit.into_iter().collect();
    }
    if let Some(u) = t.unit
        && map.creature(u).is_some_and(|c| entries.contains(&c.entry))
    {
        return vec![u];
    }
    let at = center(map, caster, t, false);
    units_around(map, at, radius.max(30.0), |g| {
        map.creature(g).is_some_and(|c| entries.contains(&c.entry))
    })
    .into_iter()
    .take(1)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_targets_thistle_tea_names() {
        assert_eq!(implicit(1), Implicit::Caster);
        assert_eq!(implicit(6), Implicit::Enemy);
        assert_eq!(implicit(22), Implicit::Location);
        assert_eq!(implicit(15), Implicit::AoeEnemyAtCaster);
        assert_eq!(implicit(61), Implicit::RaidAndClass);
        assert!(supported(63));
        assert!(wants_unit(21));
    }
}
