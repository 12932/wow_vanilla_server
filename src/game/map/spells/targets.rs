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

/// How far a chain jumps (mangos's `CHAIN_SPELL_JUMP_RADIUS`).
pub const CHAIN_JUMP_YARDS: f32 = 10.0;
/// How far a heal's chain jumps between friends (Chain Heal): further than
/// mangos's 10, your call of 2026-10-20.
pub const HEAL_CHAIN_JUMP_YARDS: f32 = 12.5;

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

/// `g`'s group: its subgroup in a raid (all of it `raid_wide`), or `g`
/// alone.
pub(crate) fn members(map: &Map, g: Guid, raid_wide: bool) -> Vec<Guid> {
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

/// Whether a party member's spell (target 35) from `caster` may land on
/// `target`: never the caster, but its master, its own pet, or one of its
/// master's group or their pets (mangos's `TARGET_UNIT_PARTY`, Thistle
/// Tea's `party_unit_allowed?`).
pub fn party_unit(map: &Map, caster: Guid, target: Guid) -> bool {
    if target == caster || !alive(map, target) {
        return false;
    }
    let owner = crate::game::map::pets::master(map, caster);
    let target_owner = crate::game::map::pets::master(map, target);
    target == owner
        || (guid::is_player(target_owner) && members(map, owner, true).contains(&target_owner))
}

/// The group `about` is of, with its pets: its master's subgroup (or the
/// whole raid), or its master alone, the members `keep` keeps and each
/// one's pet (mangos's `FillRaidOrPartyTargets` with its pets, Thistle
/// Tea's `party_pet?`). A pet's group is its master's.
fn group_units(
    map: &Map,
    about: Guid,
    raid_wide: bool,
    mut keep: impl FnMut(Guid) -> bool,
) -> Vec<Guid> {
    let owner = crate::game::map::pets::master(map, about);
    let mut out = Vec::new();
    for g in members(map, owner, raid_wide) {
        if !keep(g) {
            continue;
        }
        out.push(g);
        if let Some(pet) = crate::game::map::pets::pet_of(map, g) {
            out.push(pet);
        }
    }
    out
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

/// Whether `g` is in front of `caster`, within `degrees`; behind it, for a
/// negative cone (Onyxia's Tail Sweep's -120, as mangos's `PUSH_CONE` takes
/// it with `isInBack`).
pub(crate) fn in_cone(map: &Map, caster: Guid, g: Guid, degrees: i32) -> bool {
    let (Some(me), Some(p)) = (map.unit_view(caster), map.position_of(g)) else {
        return false;
    };
    let facing = if degrees < 0 {
        me.orientation + std::f32::consts::PI
    } else {
        me.orientation
    };
    let degrees = degrees.abs();
    let a = (p.y - me.position.y).atan2(p.x - me.position.x) - facing;
    let a = a.sin().atan2(a.cos()).abs();
    a <= degrees as f32 * std::f32::consts::PI / 360.0
}

/// The point an effect's area is at: about the caster (or the unit it is
/// about), or at the destination.
fn center(map: &Map, caster: Guid, t: &Targets, at_dest: bool) -> Vector3d {
    let caster_at = map
        .position_of(t.origin.unwrap_or(caster))
        .unwrap_or(Vector3d {
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
    // A summon or a ground's effect at a script area's place is the
    // place's, not each unit's there (Thistle Tea's destination effects).
    if matches!(e.target_a, 7 | 8) && place_effect(s, index) {
        return Vec::new();
    }
    // A unit's place (53, 63) is where the unit is now, whatever place was
    // sent (mangos's `TARGET_LOCATION_CASTER_TARGET_POSITION` setting the
    // destination to its unit, Thistle Tea's unit locations).
    let at_unit;
    let t = if matches!(e.target_a, 53 | 63)
        && let Some(p) = t.unit.and_then(|u| map.position_of(u))
    {
        at_unit = Targets {
            dest: Some(p),
            ..*t
        };
        &at_unit
    } else {
        t
    };
    // Areas and groups about the caster are about the unit a triggering
    // aura is on, and their friends and foes are that unit's (mangos's
    // `PickTargetsForSpellTrigger`, Thistle Tea's trigger resolution).
    let about = t.origin.unwrap_or(caster);
    // An area spares the creature types its spell doesn't name (Holy
    // Wrath's undead and demons), as Thistle Tea's target resolver does.
    let typed = |g: Guid| super::checks::unit_type_allowed(map, s, g);
    let enemy = |g: Guid| alive(map, g) && combat::can_attack_in_area(map, about, g) && typed(g);
    let ally = |g: Guid| {
        alive(map, g)
            && (g == about
                || (!combat::can_attack(map, about, g)
                    && !combat::hostile(map, about, g)
                    && typed(g)))
    };
    let mut out = match (a, b) {
        // An area named by the second target.
        (_, AoeEnemyAtCaster) | (AoeEnemyAtCaster, _) => {
            units_around(map, center(map, caster, t, false), r, |g| {
                g != about && enemy(g)
            })
        }
        // At the caster's own place (18) the destination is the caster's,
        // whatever the cast was aimed at (mangos's
        // `TARGET_LOCATION_CASTER_DEST`): a triggered Hellfire Effect about
        // the warlock, not their selection.
        (_, AoeEnemyAtDest)
        | (AoeEnemyAtDest, _)
        | (_, AoeEnemyAtChannel)
        | (AoeEnemyAtChannel, _) => {
            units_around(map, center(map, caster, t, e.target_a != 18), r, |g| {
                g != about && enemy(g)
            })
        }
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
                g != about && enemy(g) && in_cone(map, caster, g, degrees)
            })
        }
        (_, AoeAllyAtSource) | (AoeAllyAtSource, _) => {
            units_around(map, center(map, caster, t, false), r, ally)
        }
        (_, AoeAllyAtDest) | (AoeAllyAtDest, _) => {
            units_around(map, center(map, caster, t, e.target_a != 18), r, ally)
        }
        // The caster's party and its pets: a pet's, its master's.
        (PartyAroundCaster, _) | (_, PartyAroundCaster) => {
            let at = center(map, caster, t, e.target_a == 34);
            group_units(map, about, false, |_| true)
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
        // The raid and its pets within the effect's radius, or the spell's
        // range without one (Thistle Tea's raid area): not those it can't
        // help (a duel's other side), nor their pets; the one it is about
        // only for a helpful spell (mangos's `FillRaidOrPartyTargets`
        // `withcaster`), so Plague spares its carrier.
        (RaidAroundCaster, _) => {
            let at = center(map, caster, t, false);
            let reach = if r > 0.0 {
                r
            } else {
                map.realm.data.spells.range(s).1
            };
            let helpful = crate::game::map::auras::positive(s);
            group_units(map, about, true, |g| {
                g == about || (combat::can_assist(map, about, g) && !combat::hostile(map, about, g))
            })
            .into_iter()
            .filter(|g| helpful || *g != about)
            .filter(|g| alive(map, *g))
            .filter(|g| map.position_of(*g).is_some_and(|p| dist(p, at) <= reach))
            .collect()
        }
        // The party of the one aimed at (a pet's, its master's) about its
        // master, with their pets (mangos's `TARGET_UNIT_FRIEND_AND_PARTY`,
        // Thistle Tea's `target_party_guids`).
        (PartyAroundTarget, _) => {
            let target = t.unit.unwrap_or(caster);
            let owner = crate::game::map::pets::master(map, target);
            let at = map
                .position_of(owner)
                .or_else(|| map.position_of(target))
                .unwrap_or(center(map, caster, t, false));
            group_units(map, owner, false, |_| true)
                .into_iter()
                .filter(|g| alive(map, *g))
                .filter(|g| {
                    map.position_of(*g)
                        .is_some_and(|p| dist(p, at) <= r.max(30.0))
                })
                .collect()
        }
        // The raid's of the class of the one aimed at, and their pets of
        // that class (mangos's `TARGET_UNIT_RAID_AND_CLASS`: a hunter's
        // beast is a warrior's).
        (RaidAndClass, _) => {
            let target = t.unit.unwrap_or(caster);
            let class_of = |g: Guid| {
                map.unit_fields(g)
                    .map(|f| f.get_byte(crate::game::fields::UNIT_FIELD_BYTES_0, 1))
            };
            let class = class_of(target);
            let at = map
                .position_of(caster)
                .unwrap_or(center(map, caster, t, false));
            group_units(map, target, true, |_| true)
                .into_iter()
                .filter(|g| alive(map, *g))
                .filter(|g| class_of(*g) == class)
                .filter(|g| {
                    map.position_of(*g)
                        .is_some_and(|p| dist(p, at) <= r.max(100.0))
                })
                .collect()
        }
        (PartyMember, _) => t
            .unit
            .filter(|u| party_unit(map, caster, *u))
            .into_iter()
            .collect(),
        // A resurrection with no implicit target (Resurrection 2006) is at
        // the unit aimed at, as mangos's default target is.
        (None, _) if matches!(e.effect, 18 | 113) && t.unit.is_some() => {
            t.unit.into_iter().collect()
        }
        (Caster, _) | (None, _) => vec![caster],
        (Pet, _) => crate::game::map::pets::pet_of(map, caster)
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
        // Every script unit in a cone before the caster (60), as mangos's
        // `TARGET_ENUM_UNITS_SCRIPT_IN_CONE_60`.
        (ScriptUnits, _) if e.target_a == 60 => {
            let degrees = map
                .realm
                .data
                .spells
                .cones
                .get(&s.id)
                .copied()
                .unwrap_or(60);
            script_area(map, caster, center(map, caster, t, false), s, r)
                .into_iter()
                .filter(|g| *g != about && in_cone(map, caster, *g, degrees))
                .collect()
        }
        // Script units about the source (7) or the destination (8): every
        // one of them there, as Separation Anxiety and Golemagg's Trust
        // name theirs, or with none named every unit there, the caster
        // too (mangos's `TARGET_ENUM_UNITS_SCRIPT_AOE_AT_*`, Thistle Tea's
        // unrestricted areas): Coarse Dynamite's blast, a Cozy Fire's
        // warmth. About the caster's own place (18), the caster's.
        (_, ScriptUnits) | (ScriptUnits, _)
            if matches!(e.target_b, 7 | 8) || matches!(e.target_a, 7 | 8) =>
        {
            let kind = if matches!(e.target_b, 7 | 8) {
                e.target_b
            } else {
                e.target_a
            };
            let at_dest = kind == 8 && e.target_a != 18;
            script_area(map, caster, center(map, caster, t, at_dest), s, r)
        }
        (ScriptUnits, _) | (CreatureNearCaster, _) => script_units(map, caster, s, index, t, r),
        (GameObject, _) | (Location, _) => Vec::new(),
        (Other(_), _) => t.unit.into_iter().collect(),
    };
    // A second target of the caster's master adds the master to the first's
    // (mangos's `SetTargetMap` of both; Thistle Tea's mixed enemy and
    // master spells): a pet's buff at its enemy and its master.
    if b == CasterMaster
        && a != CasterMaster
        && let Some(owner) = map.creature(caster).and_then(|c| c.owner)
        && !out.contains(&owner)
    {
        out.push(owner);
    }
    limit(map, &mut out, s.max_affected_targets as usize, t.unit);
    out
}

/// Whether effect `index` of `s` is done at a place, not on a unit: a
/// summon of a creature or an object, a portal, a persistent area.
pub(crate) fn place_effect(s: &SpellTemplate, index: usize) -> bool {
    crate::game::map::pets::handles(s, index)
        && s.effects[index].effect != crate::game::map::pets::EFFECT_DISMISS_PET
        || matches!(s.effects[index].effect, 27 | 50 | 76 | 104..=107)
}

/// No more than `max` of `out` (0 for no limit), as Thistle Tea's
/// `TargetLimit` and target resolver pick them: each once, a random few
/// when there are too many, the one aimed at among them if it is there.
pub(crate) fn limit(map: &Map, out: &mut Vec<Guid>, max: usize, primary: Option<Guid>) {
    let mut seen = ahash::AHashSet::new();
    out.retain(|g| seen.insert(*g));
    if max == 0 || out.len() <= max {
        return;
    }
    if super::outcome::rolling(map) {
        use rand::seq::SliceRandom;
        out.shuffle(&mut rand::rng());
    }
    match primary.filter(|p| out.contains(p)) {
        Some(p) => {
            out.retain(|g| *g != p);
            out.truncate(max - 1);
            out.push(p);
        }
        None => out.truncate(max),
    }
}

/// How much health `g` is short of its maximum.
fn missing_health(map: &Map, g: Guid) -> u32 {
    map.unit_fields(g).map_or(0, |f| {
        f.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH)
            .saturating_sub(f.get_u32(crate::game::fields::UNIT_FIELD_HEALTH))
    })
}

/// A chain: the first target, then the next within a jump of the last,
/// up to the effect's count, each once (Thistle Tea's `ChainTargets`,
/// mangos's `SCHEME_CLOSEST_CHAIN` and `ChainHealingOrder`). A jump goes
/// only to a living unit of a type the spell names, that the caster sees
/// through any stealth, in sight of the last unless the spell ignores
/// sight: for harm, an enemy an area may hit, the nearest; for a heal, a
/// friend the caster may help who is hurt, the last one's group first,
/// then the most hurt, then the nearest. A heal may come back to its
/// caster.
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
    let ignores_sight = s.attr(2, crate::game::data::spells::ATTR_EX2_IGNORE_LOS);
    let mut out = vec![first];
    let mut last = first;
    while out.len() < count {
        let Some(at) = map.position_of(last) else {
            break;
        };
        let jump = if heal {
            HEAL_CHAIN_JUMP_YARDS
        } else {
            CHAIN_JUMP_YARDS
        };
        // The cheap tests for all within the jump, the nearest first; the
        // costly ones (whom an area may hit or the caster help, stealth, a
        // ray of sight) in the order a jump takes them, until one passes,
        // where every unit within it had them all (review 5's combat 9).
        let near = units_around(map, at, jump, |g| {
            !out.contains(&g)
                && alive(map, g)
                && super::checks::unit_type_allowed(map, s, g)
                && if heal {
                    missing_health(map, g) > 0
                } else {
                    g != caster
                }
        });
        let takes = |g: Guid| {
            (if heal {
                !combat::can_attack(map, caster, g)
                    && !combat::hostile(map, caster, g)
                    && combat::can_assist(map, caster, g)
            } else {
                combat::can_attack_in_area(map, caster, g)
            }) && super::stealth::sees(map, caster, g)
                && (ignores_sight
                    || map
                        .position_of(g)
                        .is_some_and(|p| super::checks::sight(map, at, p)))
        };
        // Nearest first already; a heal's order over that.
        let pick = if heal {
            let group = members(map, last, true);
            let mut order: Vec<(usize, Guid)> = near.into_iter().enumerate().collect();
            order.sort_by_cached_key(|(i, g)| {
                (!group.contains(g), u32::MAX - missing_health(map, *g), *i)
            });
            order.into_iter().map(|(_, g)| g).find(|g| takes(*g))
        } else {
            near.into_iter().find(|g| takes(*g))
        };
        let Some(g) = pick else {
            break;
        };
        out.push(g);
        last = g;
    }
    out
}

/// Whether `g`, of `entry`, is a target a `spell_script_target` row of
/// `kinds` names for spell `spell`, meeting the row's condition with the
/// caster as its source.
pub fn script_target(
    map: &Map,
    caster: Guid,
    spell: u32,
    kinds: &[u32],
    entry: u32,
    g: Guid,
) -> bool {
    script_target_of(map, caster, spell, kinds, entry, g, None)
}

/// [`script_target`] for effect `index` if one is named: a row that spares
/// that effect (its `inverseEffectMask`) doesn't count, as mangos's
/// `CanNotHitWithSpellEffect` skips it.
pub fn script_target_of(
    map: &Map,
    caster: Guid,
    spell: u32,
    kinds: &[u32],
    entry: u32,
    g: Guid,
    index: Option<usize>,
) -> bool {
    let data = &map.realm.data.spells;
    // A creature row (1) names the living, a dead one's (2) corpses
    // (vmangos's `SPELL_TARGET_TYPE_CREATURE` and `_DEAD`).
    let alive = map.creature(g).map(|c| c.alive());
    data.script_targets.get(&spell).is_some_and(|v| {
        v.iter().any(|(k, e)| {
            kinds.contains(k)
                && *e == entry
                && index.is_none_or(|i| {
                    data.script_target_spared
                        .get(&(spell, *k, *e))
                        .is_none_or(|m| m & (1 << i) == 0)
                })
                && match (*k, alive) {
                    (1, Some(alive)) => alive,
                    (2, Some(alive)) => !alive,
                    _ => true,
                }
                && data
                    .script_target_conditions
                    .get(&(spell, *k, *e))
                    .is_none_or(|c| {
                        crate::game::map::conditions::met(map, *c, Some(caster), Some(g))
                    })
        })
    })
}

/// The units a scripted spell aims at: the creatures `spell_script_target`
/// names, near the caster (or the target the client picked).
/// Every creature a spell's script targets name within `radius` of `at`.
fn script_area(map: &Map, caster: Guid, at: Vector3d, s: &SpellTemplate, radius: f32) -> Vec<Guid> {
    if !map.realm.data.spells.script_targets.contains_key(&s.id) {
        return units_around(map, at, radius, |g| alive(map, g));
    }
    units_around(map, at, radius, |g| {
        map.creature(g)
            .is_some_and(|c| script_target(map, caster, s.id, &[1, 2], c.entry, g))
    })
}

/// Where effect `index` of `s` at target 46 is (mangos's
/// `TARGET_LOCATION_SCRIPT_NEAR_CASTER`): at the nearest creature its
/// script targets name, else the nearest object, within reach of the
/// caster; none if it names none there.
pub(crate) fn script_place(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
) -> Option<Vector3d> {
    if !map.realm.data.spells.script_targets.contains_key(&s.id) {
        return None;
    }
    let at = map.position_of(caster)?;
    let radius = map.realm.data.spells.radius(s.effects[index].radius_index);
    let reach = script_reach(map, s, index, radius);
    let nearest = |of: &mut dyn Iterator<Item = (Vector3d, u32, Guid)>, kinds: &[u32]| {
        of.filter(|(p, entry, g)| {
            dist(*p, at) <= reach && script_target(map, caster, s.id, kinds, *entry, *g)
        })
        .map(|(p, ..)| p)
        .min_by(|a, b| dist(*a, at).total_cmp(&dist(*b, at)))
    };
    let around: Vec<Guid> = map.grid.around(at.x, at.y, reach).collect();
    let creature = nearest(
        &mut around
            .iter()
            .filter_map(|g| map.creature(*g))
            .map(|c| (c.position, c.entry, c.guid)),
        &[1, 2],
    );
    creature.or_else(|| {
        nearest(
            &mut around
                .iter()
                .filter(|g| guid::is_game_object(**g))
                .filter_map(|g| map.gameobject(*g))
                .map(|o| (o.position, o.entry, o.guid)),
            &[0],
        )
    })
}

/// How far a script's creature is looked for: the effect's radius, else
/// the spell's range, else as far as one sees (mangos's
/// `GetSpellRangeAndRadius` and `CheckScriptTargeting`).
fn script_reach(map: &Map, s: &SpellTemplate, index: usize, radius: f32) -> f32 {
    let reach = if s.effects[index].radius_index != 0 {
        radius
    } else {
        map.realm.data.spells.range(s).1
    };
    if reach > 0.0 { reach } else { 100.0 }
}

/// A unit's size, its bounding radius.
fn bounding(map: &Map, g: Guid) -> f32 {
    map.unit_fields(g)
        .map_or(0.0, |f| {
            f.get_f32(crate::game::fields::UNIT_FIELD_BOUNDINGRADIUS)
        })
        .max(0.0)
}

/// Whether a spell names creatures for its effect `index` to look for
/// (`spell_script_target` rows of the living or the dead).
fn names_creatures(map: &Map, s: &SpellTemplate) -> bool {
    map.realm
        .data
        .spells
        .script_targets
        .get(&s.id)
        .is_some_and(|v| v.iter().any(|(k, _)| *k == 1 || *k == 2))
}

/// The creature a scripted spell's effect `index` aims at: the one picked
/// if it is one of those named, else the nearest within reach of both
/// edges (mangos's `CheckScriptTargeting`, Thistle Tea's `SpellUnits`).
fn script_units(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    t: &Targets,
    radius: f32,
) -> Vec<Guid> {
    if !names_creatures(map, s) {
        return t.unit.into_iter().collect();
    }
    let reach = script_reach(map, s, index, radius);
    let at = center(map, caster, t, false);
    let mine = bounding(map, t.origin.unwrap_or(caster));
    let fits = |g: Guid| {
        map.creature(g).is_some_and(|c| {
            script_target(map, caster, s.id, &[1, 2], c.entry, g)
                && dist(c.position, at) <= reach + mine + bounding(map, g)
        })
    };
    if let Some(u) = t.unit
        && fits(u)
    {
        return vec![u];
    }
    // Wide enough for the biggest creature's edge.
    units_around(map, at, reach + mine + 10.0, fits)
        .into_iter()
        .take(1)
        .collect()
}

/// Whether a cast's effect at a script's creature near its caster (38)
/// finds none of those its spell names, which refuses it (mangos's
/// `CheckScriptTargeting`'s `SPELL_FAILED_BAD_TARGETS`, Thistle Tea's
/// `UnitTargets` error).
pub(crate) fn script_creature_missing(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    m: &Mods,
) -> bool {
    names_creatures(map, s)
        && (0..3).any(|i| {
            let e = &s.effects[i];
            e.effect != 0
                && e.target_a == 38
                && script_units(map, caster, s, i, t, radius(map, s, i, m)).is_empty()
        })
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
