//! vmangos's `conditions`: whether a condition holds for a source (the
//! creature, object or script owner) and a target (usually a player).
//! Combinations (and, or, not), the reverse and swap flags, and the leaf
//! types Thistle Tea evaluates. Each comes to met, unmet or unknown, as
//! Thistle Tea's `Condition.Result` has it: what this server can't judge (a
//! type it doesn't know, a fact its source or target hasn't got) stays
//! unknown through a not or a reverse, and an unknown gate stays shut.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::scripts::{CF_REVERSE_RESULT, CF_SWAP_TARGETS, Condition};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::player::Player;
use crate::game::registry::Team;

/// What a condition comes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truth {
    Met,
    Unmet,
    /// Not judged: a type or a fact this server hasn't got.
    Unknown,
}

impl Truth {
    pub fn of(b: bool) -> Self {
        if b { Truth::Met } else { Truth::Unmet }
    }

    /// Met for unmet and the other way about; unknown stays unknown.
    pub fn negate(self) -> Self {
        match self {
            Truth::Met => Truth::Unmet,
            Truth::Unmet => Truth::Met,
            Truth::Unknown => Truth::Unknown,
        }
    }

    /// Met if any is; else unknown if any is; else unmet.
    pub fn any(results: impl IntoIterator<Item = Truth>) -> Self {
        let mut out = Truth::Unmet;
        for r in results {
            match r {
                Truth::Met => return Truth::Met,
                Truth::Unknown => out = Truth::Unknown,
                Truth::Unmet => {}
            }
        }
        out
    }

    /// Unmet if any is; else unknown if any is; else met.
    pub fn all(results: impl IntoIterator<Item = Truth>) -> Self {
        let mut out = Truth::Met;
        for r in results {
            match r {
                Truth::Unmet => return Truth::Unmet,
                Truth::Unknown => out = Truth::Unknown,
                Truth::Met => {}
            }
        }
        out
    }
}

impl From<Option<bool>> for Truth {
    fn from(b: Option<bool>) -> Self {
        b.map_or(Truth::Unknown, Truth::of)
    }
}

/// Whether condition `id` holds; 0 always does. Unknown is no.
pub fn met(map: &Map, id: u32, source: Option<Guid>, target: Option<Guid>) -> bool {
    check(map, id, source, target) == Truth::Met
}

/// What condition `id` comes to; 0 is met, one with no row unknown.
pub fn check(map: &Map, id: u32, source: Option<Guid>, target: Option<Guid>) -> Truth {
    if id == 0 {
        return Truth::Met;
    }
    eval(map, id, source, target, 0)
}

fn eval(map: &Map, id: u32, source: Option<Guid>, target: Option<Guid>, depth: u32) -> Truth {
    if depth > 16 {
        return Truth::Unknown;
    }
    let Some(c) = map.realm.data.scripts.conditions.get(&id).copied() else {
        return Truth::Unknown;
    };
    let (source, target) = if c.flags & CF_SWAP_TARGETS != 0 {
        (target, source)
    } else {
        (source, target)
    };
    let result = leaf(map, &c, source, target, depth);
    if c.flags & CF_REVERSE_RESULT != 0 {
        result.negate()
    } else {
        result
    }
}

/// Note where a creature or object stands, for the area condition: its
/// zone and area, worked out again if `again`, else only if not yet known.
pub fn locate(map: &mut Map, g: Guid, again: bool) {
    let at = match (map.creature(g), map.gameobject(g)) {
        (Some(c), _) if again || c.zone_area.is_none() => c.position,
        (None, Some(o)) if again || o.zone_area.is_none() => o.position,
        _ => return,
    };
    let Some(found) = crate::game::map::world::zone_and_area(map, at) else {
        return;
    };
    if let Some(c) = map.creature_mut(g) {
        c.zone_area = Some(found);
    } else if let Some(o) = map.gameobject_mut(g) {
        o.zone_area = Some(found);
    }
}

/// Whether an escort the world has under way says it failed: the hook the
/// escort quests fill in. None does yet.
pub fn escort_failed(_map: &Map, _source: Option<Guid>, _target: Option<Guid>) -> bool {
    false
}

/// A comparison by mode: 0 equal, 1 at least, 2 at most; another mode
/// isn't judged.
fn compare(actual: i64, expected: i64, mode: i32) -> Truth {
    match mode {
        0 => Truth::of(actual == expected),
        1 => Truth::of(actual >= expected),
        2 => Truth::of(actual <= expected),
        _ => Truth::Unknown,
    }
}

fn pct_of(cur: u32, max: u32) -> i64 {
    i64::from(cur) * 100 / i64::from(max.max(1))
}

/// The zone and area of `g`: `None` if it isn't here, `Some(None)` if it
/// is but they aren't known yet.
fn zone_area(map: &Map, g: Guid) -> Option<Option<(u32, u32)>> {
    if let Some(p) = map.players.get(&g) {
        return Some(Some((p.zone, p.area)));
    }
    if let Some(c) = map.creature(g) {
        return Some(c.zone_area);
    }
    map.gameobject(g).map(|o| o.zone_area)
}

/// Whether spawn `spawn` is one of this map's objects.
fn listed_object(map: &Map, spawn: u32) -> Option<&crate::game::data::GameObjectSpawn> {
    map.realm
        .data
        .gameobject_spawns
        .get(&map.key.map)
        .and_then(|v| v.iter().find(|s| s.guid == spawn))
}

/// An object's loot state, as vmangos numbers it: 1 ready, 2 in use (a
/// chest opened, a door or button not back at rest).
fn loot_state(map: &Map, o: &crate::game::map::gameobject::GameObject) -> i32 {
    let used = if o.kind == crate::game::map::objects::GO_CHEST {
        o.loot.is_some()
    } else {
        o.fields.get_u32(GAMEOBJECT_STATE) != crate::game::map::objects::resting_state(map, o.guid)
    };
    if used { 2 } else { 1 }
}

/// Whether an Argent Dawn commission is on `p`: an aura whose spell has
/// the commission's look and is an ability or works mounted.
fn commissioned(map: &Map, p: &Player) -> bool {
    use crate::game::data::spells::{ATTR_ABILITY, ATTR_ALLOW_WHILE_MOUNTED};
    p.auras.holders.iter().any(|h| {
        map.realm.data.spells.get(h.spell).is_some_and(|s| {
            s.spell_visual == ARGENT_DAWN_COMMISSION_VISUAL
                && (s.attr(0, ATTR_ABILITY) || s.attr(0, ATTR_ALLOW_WHILE_MOUNTED))
        })
    })
}

const ARGENT_DAWN_COMMISSION_VISUAL: u32 = 3580;

fn leaf(map: &Map, c: &Condition, source: Option<Guid>, target: Option<Guid>, depth: u32) -> Truth {
    let [v1, v2, v3, v4] = c.values;
    let present = |g: Option<Guid>| g.filter(|g| map.position_of(*g).is_some());
    // The facts of a player target; another target hasn't got them.
    let player = |f: &dyn Fn(&Player) -> bool| -> Truth {
        target.and_then(|t| map.players.get(&t)).map(f).into()
    };
    let unit_fields = |g: Option<Guid>| g.and_then(|g| map.unit_fields(g));
    let position = |g: Option<Guid>| g.and_then(|g| map.position_of(g));
    let child = |id: i32, source: Option<Guid>, target: Option<Guid>| {
        if id > 0 {
            eval(map, id as u32, source, target, depth + 1)
        } else {
            Truth::Unknown
        }
    };
    let children = || c.values.iter().copied().filter(|v| *v > 0);
    match c.kind {
        // Not, or, and.
        -3 => child(v1, source, target).negate(),
        -2 if children().next().is_some() => {
            Truth::any(children().map(|v| child(v, source, target)))
        }
        -1 if children().next().is_some() => {
            Truth::all(children().map(|v| child(v, source, target)))
        }
        0 => Truth::Met,
        // An aura, by spell (and effect).
        1 => target
            .and_then(|t| crate::game::map::auras::auras(map, t))
            .map(|a| {
                a.holders.iter().any(|h| {
                    h.spell == v1 as u32
                        && (v2 < 0 || h.effects.iter().any(|e| i32::from(e.effect) == v2))
                })
            })
            .into(),
        // Items carried (not banked), worn.
        2 => player(&|p| p.carried_count_of(v1 as u32) >= v2.max(1) as u32),
        3 => player(&|p| {
            p.items.iter().any(|i| {
                i.entry == v1 as u32
                    && i.bag == crate::game::store::BAG_PLAYER
                    && i.slot < crate::game::map::player::EQUIPMENT_SLOTS
            })
        }),
        // Zone or area: the source's, else the target's.
        4 => {
            let of = |g: Option<Guid>| g.and_then(|g| zone_area(map, g));
            let at = match of(source) {
                Some(Some(za)) => Some(za),
                // Not worked out yet: the one it deals with stands by it.
                Some(None) => of(target).flatten(),
                None => of(target).flatten(),
            };
            at.map(|(z, a)| z == v1 as u32 || a == v1 as u32).into()
        }
        // Reputation: the rank with a faction, at least or at most.
        5 => player(&|p| {
            crate::game::map::reputation::rank(p, &map.realm.data, v1.max(0) as u32) as i32 >= v2
        }),
        30 => player(&|p| {
            crate::game::map::reputation::rank(p, &map.realm.data, v1.max(0) as u32) as i32 <= v2
        }),
        6 => player(&|p| {
            let team = match p.team {
                Team::Alliance => 469,
                Team::Horde => 67,
            };
            team == v1
        }),
        7 => player(&|p| {
            crate::game::map::skills::value(p, v1 as u32).is_some_and(|s| s >= v2.max(0) as u32)
        }),
        // Below a skill value; with value 1, not knowing the skill.
        29 => player(&|p| match crate::game::map::skills::value(p, v1 as u32) {
            None => v2 == 1,
            Some(s) => v2 != 1 && s < v2.max(0) as u32,
        }),
        8 => player(&|p| p.journal.rewarded.contains(&(v1 as u32))),
        // Taken: 0 either way, 1 incomplete, 2 complete; failed is neither.
        9 => player(&|p| {
            let Some(slot) = p.journal.slot_of(v1 as u32) else {
                return false;
            };
            if p.journal.log[slot].failed {
                return false;
            }
            let done = map
                .realm
                .data
                .npc
                .quests
                .get(&(v1 as u32))
                .is_some_and(|q| crate::game::map::quests::complete(p, q, &p.journal.log[slot]));
            match v2 {
                0 => true,
                1 => !done,
                2 => done,
                _ => false,
            }
        }),
        10 => player(&|p| commissioned(map, p)),
        22 => player(&|p| !p.journal.has(v1 as u32) && !p.journal.rewarded.contains(&(v1 as u32))),
        // Available: to be taken (mangos's `CanTakeQuest`), its event on
        // and its own condition met, and a breadcrumb's targets' too (an
        // unknown among them unknown).
        19 => match map.realm.data.npc.quests.get(&(v1 as u32)) {
            Some(q) => {
                let data = &map.realm.data;
                let offered = |quest: u32| {
                    let own = data
                        .scripts
                        .quest_condition
                        .get(&quest)
                        .map_or(Truth::Met, |c| child(*c as i32, source, target));
                    Truth::all([
                        Truth::of(crate::game::events::quest_offered(&map.realm, quest)),
                        own,
                    ])
                };
                let targets: &[u32] = data
                    .npc
                    .quest_graph()
                    .breadcrumb_targets
                    .get(&q.id)
                    .map_or(&[], Vec::as_slice);
                Truth::all(
                    [player(&|p| {
                        !p.journal.has(q.id)
                            && !p.journal.rewarded.contains(&q.id)
                            && crate::game::map::quests::can_take(p, q, data)
                    })]
                    .into_iter()
                    .chain([offered(q.id)])
                    .chain(targets.iter().map(|t| offered(*t))),
                )
            }
            None => Truth::Unknown,
        },
        12 => Truth::of(map.realm.social.events.lock().contains(&(v1 as u32))),
        // Race and class masks.
        14 => player(&|p| {
            let mask = |m: i32, v: u8| m == 0 || (m as u32) & (1 << (v.max(1) - 1)) != 0;
            mask(v1, p.row.race) && mask(v2, p.row.class)
        }),
        15 => unit_fields(target).map_or(Truth::Unknown, |f| {
            compare(i64::from(f.get_u32(UNIT_FIELD_LEVEL)), i64::from(v1), v2)
        }),
        // The source is one of these entries.
        16 => present(source).map_or(Truth::Unknown, |s| {
            let entry = map
                .creature(s)
                .map(|c| c.entry)
                .or_else(|| map.gameobject(s).map(|o| o.entry))
                .unwrap_or(0) as i32;
            Truth::of(
                entry == v1
                    || (v2 != 0 && entry == v2)
                    || (v3 != 0 && entry == v3)
                    || (v4 != 0 && entry == v4),
            )
        }),
        17 => match v2 {
            0 | 1 => player(&|p| p.spells.contains(&(v1 as u32)) == (v2 == 0)),
            _ => Truth::Unknown,
        },
        // A creature near the target (or the source), alive, or with
        // value 3, dead or gone; with value 4, not the target itself.
        20 => {
            let Some(at) = position(target).or_else(|| position(source)) else {
                return Truth::Unknown;
            };
            let found =
                crate::game::map::scripts::creatures_near(map, at, v1 as u32, v2 as f32, false)
                    .into_iter()
                    .filter(|g| v4 == 0 || Some(*g) != target)
                    .any(|g| map.creature(g).is_some_and(|c| c.alive()) == (v3 == 0));
            Truth::of(found)
        }
        // An object near the target (or the source).
        21 => {
            let Some(at) = position(target).or_else(|| position(source)) else {
                return Truth::Unknown;
            };
            Truth::of(
                map.gameobjects
                    .iter()
                    .any(|(_, o)| o.entry == v1 as u32 && dist(o.position, at) <= v2 as f32),
            )
        }
        23 => player(&|p| p.count_of(v1 as u32) >= v2.max(1) as u32),
        24 => compare(
            i64::from(crate::game::data::PATCH as i32),
            i64::from(v1),
            v2,
        ),
        27 => unit_fields(target).map_or(Truth::Unknown, |f| {
            Truth::of(i32::from(f.get_byte(UNIT_FIELD_BYTES_0, 2)) == v1)
        }),
        // A player; with value 1, or a player's.
        28 => match present(target) {
            None => Truth::Unknown,
            Some(t) if guid::is_player(t) => Truth::Met,
            Some(t) => match v1 {
                0 => Truth::Unmet,
                1 => Truth::of(
                    map.creature(t)
                        .is_some_and(|c| c.owner.is_some_and(guid::is_player)),
                ),
                _ => Truth::Unknown,
            },
        },
        // A field of the source has a flag.
        31 => source
            .and_then(|s| {
                map.unit_fields(s)
                    .or_else(|| map.gameobject(s).map(|o| &o.fields))
            })
            .map(|f| f.get_u32(v1 as u16) & v2 as u32 != 0)
            .into(),
        33 => Truth::of(map.key.map == v1 as u32),
        11 => compare(
            i64::from(crate::game::map::scripts::server_variable(v1 as u32)),
            i64::from(v2),
            v3,
        ),
        35 => match crate::game::map::map_events::data(map, v1 as u32, v2 as u32) {
            Some(d) => compare(i64::from(d), i64::from(v3), v4),
            None => Truth::Unmet,
        },
        36 => Truth::of(crate::game::map::map_events::active(map, v1 as u32)),
        // Every target of a map event still here meets the child.
        47 if v2 > 0 => Truth::all(
            crate::game::map::map_events::targets(map, v1 as u32)
                .into_iter()
                .map(|t| {
                    if map.position_of(t).is_some() {
                        child(v2, source, Some(t))
                    } else {
                        Truth::Met
                    }
                }),
        ),
        // The copy's script's data: unmet with no script, unknown for one
        // not ported.
        34 => match map.instance.script {
            None => Truth::Unmet,
            Some(crate::game::map::instance_script::Script::Unported) => Truth::Unknown,
            Some(_) => compare(
                i64::from(crate::game::map::instance_script::get_data(map, v1 as u32)),
                i64::from(v2),
                v3,
            ),
        },
        // Line of sight from the source's eyes to the target's middle.
        37 => match (position(source), position(target)) {
            (Some(a), Some(b)) => {
                let up = |v: Vector3d, h: f32| Vector3d { z: v.z + h, ..v };
                Truth::of(
                    map.nav
                        .as_ref()
                        .is_none_or(|n| n.sight(up(a, 2.0), up(b, 1.0))),
                )
            }
            _ => Truth::Unknown,
        },
        38 => match (position(source), position(target)) {
            (Some(a), Some(b)) => compare(dist(a, b) as i64, i64::from(v1), v2),
            _ => Truth::Unknown,
        },
        39 => match target {
            Some(t) if map.players.contains_key(&t) => Truth::of(
                crate::game::map::spells::checks::moving(&map.players[&t].movement.flags),
            ),
            Some(t) => map
                .creature(t)
                .map(|c| matches!(c.motion, crate::game::map::creature::Motion::Moving(_)))
                .into(),
            None => Truth::Unknown,
        },
        40 => target
            .filter(|t| map.unit_view(*t).is_some())
            .map(|t| crate::game::map::pets::pet_of(map, t).is_some())
            .into(),
        41 => {
            unit_fields(target).map_or(Truth::Unknown, |f| match f.get_u32(UNIT_FIELD_MAXHEALTH) {
                0 => Truth::Unknown,
                max => compare(pct_of(f.get_u32(UNIT_FIELD_HEALTH), max), i64::from(v1), v2),
            })
        }
        42 => unit_fields(target).map_or(Truth::Unknown, |f| {
            let pct = match f.get_u32(UNIT_FIELD_MAXPOWER1) {
                0 => 100,
                max => pct_of(f.get_u32(UNIT_FIELD_POWER1), max),
            };
            compare(pct, i64::from(v1), v2)
        }),
        43 => unit_fields(target)
            .map(|f| {
                f.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                )
            })
            .into(),
        // How the target regards the source, as a reputation rank.
        44 => match (source, target) {
            (Some(s), Some(t)) => crate::game::map::reputation::reaction_rank(map, t, s)
                .map_or(Truth::Unknown, |r| compare(i64::from(r), i64::from(v1), v2)),
            _ => Truth::Unknown,
        },
        45 => player(&|p| map.realm.social.groups.lock().group_of(p.guid).is_some()),
        46 => target
            .and_then(|t| map.unit_view(t))
            .map(|u| u.alive)
            .into(),
        // An object spawned; one of the map's spawns that is gone isn't.
        48 => match target {
            Some(t) if map.gameobject(t).is_some() => Truth::Met,
            Some(t)
                if guid::is_game_object(t) && listed_object(map, guid::counter(t)).is_some() =>
            {
                Truth::Unmet
            }
            _ => Truth::Unknown,
        },
        49 => target
            .and_then(|t| map.gameobject(t))
            .map(|o| loot_state(map, o) == v1)
            .into(),
        // The child on the object of spawn value 1, gone or not; one not on
        // this map doesn't.
        50 if v2 > 0 => {
            if let Some(o) = crate::game::map::scripts::object_of_spawn(map, v1 as u32) {
                return child(v2, source, Some(o));
            }
            match listed_object(map, v1 as u32) {
                Some(s) => child(v2, source, Some(guid::game_object(s.entry, s.guid))),
                None => Truth::Unmet,
            }
        }
        // The rank as it is shown (Private 1 to Grand Marshal 14).
        51 => match present(target) {
            Some(t) => match map.players.get(&t) {
                Some(p) => compare(
                    i64::from(p.fields.get_byte(PLAYER_BYTES_3, 3).saturating_sub(4)),
                    i64::from(v1),
                    v2,
                ),
                None => Truth::Unmet,
            },
            None => Truth::Unknown,
        },
        // The source's spawn is one of these.
        52 => source
            .and_then(|s| {
                map.creature(s)
                    .and_then(|c| c.spawn_guid)
                    .or_else(|| map.gameobject(s).and_then(|o| o.spawn_guid))
            })
            .map(|g| {
                let g = g as i32;
                g == v1 || (v2 != 0 && g == v2) || (v3 != 0 && g == v3) || (v4 != 0 && g == v4)
            })
            .into(),
        53 => {
            use chrono::Timelike;
            let now = chrono::Local::now();
            let at = (now.hour() as i32, now.minute() as i32);
            Truth::of(at >= (v1, v2) && at <= (v3, v4))
        }
        // The target within value 4 of a point.
        54 => {
            let [x, y, z, r] = map
                .realm
                .data
                .scripts
                .condition_floats
                .get(&c.entry)
                .copied()
                .unwrap_or(c.values.map(|v| v as f32));
            // Jaina in her city measures from its stand-in for her tower.
            let [x, y, z] = crate::game::jaina::stand_in(map, target, [x, y, z]);
            position(target)
                .map(|at| dist(at, Vector3d { x, y, z }) <= r)
                .into()
        }
        55 => target
            .and_then(|t| map.gameobject(t))
            .map(|o| o.fields.get_u32(GAMEOBJECT_STATE) as i32 == v1)
            .into(),
        // A player within value 2 of the target: any (0), one the target
        // is hostile (1) or friendly (2) to.
        56 => {
            let (Some(t), Some(at)) = (target, position(target)) else {
                return Truth::Unknown;
            };
            let mut near = map
                .players
                .values()
                .filter(|p| dist(p.position(), at) <= v2 as f32);
            match v1 {
                0 => Truth::of(near.next().is_some()),
                1 | 2 => Truth::any(near.map(|p| {
                    crate::game::map::reputation::reaction_rank(map, t, p.guid)
                        .map(|r| {
                            if v1 == 1 {
                                r <= crate::game::data::misc::RANK_HOSTILE
                            } else {
                                r >= crate::game::data::misc::RANK_FRIENDLY
                            }
                        })
                        .into()
                })),
                _ => Truth::Unknown,
            }
        }
        57 => present(source).map_or(Truth::Unknown, |s| {
            let leader = crate::game::map::creature_groups::group_leader(map, s);
            Truth::of(leader.is_some() && (v1 == 0 || leader == Some(v1 as u32)))
        }),
        58 => present(source).map_or(Truth::Unknown, |s| {
            Truth::of(crate::game::map::creature_groups::all_dead(map, s))
        }),
        59 => player(&|p| {
            map.realm.data.area(v1 as u32).is_some_and(|a| {
                let bit = a.explore_bit;
                let word = PLAYER_EXPLORED_ZONES_1 + (bit / 32) as u16;
                p.fields.get_u32(word) & (1 << (bit % 32)) != 0
            })
        }),
        // An escort's failure: the source (0x1) or target (0x2) dead, or
        // them farther apart than value 2.
        25 => {
            let dead = |g: Option<Guid>| -> Truth {
                g.and_then(|g| map.unit_view(g)).map(|u| !u.alive).into()
            };
            let mut parts = vec![Truth::of(escort_failed(map, source, target))];
            if v1 & 0x1 != 0 {
                parts.push(dead(source));
            }
            if v1 & 0x2 != 0 {
                parts.push(dead(target));
            }
            if v2 > 0 {
                parts.push(match (position(source), position(target)) {
                    (Some(a), Some(b)) => Truth::of(dist(a, b) > v2 as f32),
                    _ => Truth::Unknown,
                });
            }
            Truth::any(parts)
        }
        // A holiday: one of the game events under way is it.
        26 => {
            let holidays = &map.realm.data.scripts.event_holidays;
            Truth::of(
                map.realm
                    .social
                    .events
                    .lock()
                    .iter()
                    .any(|e| holidays.get(e).copied() == Some(v1 as u32)),
            )
        }
        // Thistle Tea's: the target's companion is of entry value1 (or is
        // any, for 0).
        MINI_PET => player(&|p| {
            p.owned
                .critter
                .and_then(|g| map.creature(g))
                .is_some_and(|c| v1 <= 0 || c.entry == v1 as u32)
        }),
        // The instance scripts' own conditions, a path to the victim, the
        // last waypoint, a combination with nothing in it, and a type
        // Thistle Tea doesn't know aren't judged.
        _ => Truth::Unknown,
    }
}

/// Thistle Tea's condition with no vmangos number: the target's
/// companion.
pub const MINI_PET: i32 = 200;
