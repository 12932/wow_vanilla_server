//! vmangos's `conditions`: whether a condition holds for a source (the
//! creature, object or script owner) and a target (usually a player).
//! Combinations (and, or, not), the reverse and swap flags, and the leaf
//! types Thistle Tea evaluates. A condition this server can't judge fails,
//! as an unknown result gates an event or step closed.

use wow_world_messages::Guid;

use crate::game::data::scripts::{CF_REVERSE_RESULT, CF_SWAP_TARGETS, Condition};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::registry::Team;

/// Whether condition `id` holds; 0 always does.
pub fn met(map: &Map, id: u32, source: Option<Guid>, target: Option<Guid>) -> bool {
    if id == 0 {
        return true;
    }
    eval(map, id, source, target, 0)
}

fn eval(map: &Map, id: u32, source: Option<Guid>, target: Option<Guid>, depth: u32) -> bool {
    if depth > 16 {
        return false;
    }
    let Some(c) = map.realm.data.scripts.conditions.get(&id).copied() else {
        return false;
    };
    let (source, target) = if c.flags & CF_SWAP_TARGETS != 0 {
        (target, source)
    } else {
        (source, target)
    };
    let result = leaf(map, &c, source, target, depth);
    if c.flags & CF_REVERSE_RESULT != 0 {
        !result
    } else {
        result
    }
}

fn compare(actual: i64, expected: i64, mode: i32) -> bool {
    match mode {
        1 => actual >= expected,
        2 => actual <= expected,
        _ => actual == expected,
    }
}

fn pct_of(cur: u32, max: u32) -> i64 {
    if max == 0 {
        100
    } else {
        i64::from(cur) * 100 / i64::from(max)
    }
}

fn leaf(map: &Map, c: &Condition, source: Option<Guid>, target: Option<Guid>, depth: u32) -> bool {
    let [v1, v2, v3, v4] = c.values;
    let player = target.and_then(|t| map.players.get(&t));
    let unit_fields = |g: Option<Guid>| g.and_then(|g| map.unit_fields(g));
    let creature = |g: Option<Guid>| g.and_then(|g| map.creature(g));
    match c.kind {
        // Not, or, and.
        -3 => !eval(map, v1.max(0) as u32, source, target, depth + 1),
        -2 => c
            .values
            .iter()
            .filter(|v| **v > 0)
            .any(|v| eval(map, *v as u32, source, target, depth + 1)),
        -1 => c
            .values
            .iter()
            .filter(|v| **v > 0)
            .all(|v| eval(map, *v as u32, source, target, depth + 1)),
        0 => true,
        // An aura, by spell (and effect).
        1 => target.is_some_and(|t| {
            crate::game::map::auras::auras(map, t).is_some_and(|a| {
                a.holders.iter().any(|h| {
                    h.spell == v1 as u32
                        && (v2 < 0 || h.effects.iter().any(|e| i32::from(e.effect) == v2))
                })
            })
        }),
        2 => player.is_some_and(|p| p.count_of(v1 as u32) >= v2.max(1) as u32),
        3 => player.is_some_and(|p| {
            p.items.iter().any(|i| {
                i.entry == v1 as u32
                    && i.bag == crate::game::store::BAG_PLAYER
                    && i.slot < crate::game::map::player::EQUIPMENT_SLOTS
            })
        }),
        // Zone or area.
        4 => {
            let of = |g: Option<Guid>| {
                g.and_then(|g| map.players.get(&g))
                    .map(|p| (p.zone, p.area))
            };
            of(source)
                .or_else(|| of(target))
                .is_some_and(|(z, a)| z == v1 as u32 || a == v1 as u32)
        }
        // Reputation at least: without reputation tracked, neutral (3).
        5 => player.is_some_and(|_| 3 >= v2),
        30 => player.is_some_and(|_| 3 <= v2),
        6 => player.is_some_and(|p| {
            let team = match p.team {
                Team::Alliance => 469,
                Team::Horde => 67,
            };
            team == v1
        }),
        7 => player.is_some_and(|p| {
            crate::game::map::skills::value(p, v1 as u32).is_some_and(|s| s >= v2.max(0) as u32)
        }),
        29 => player.is_some_and(|p| match crate::game::map::skills::value(p, v1 as u32) {
            None => true,
            Some(s) => v2 != 1 && s < v2.max(0) as u32,
        }),
        8 => player.is_some_and(|p| p.journal.rewarded.contains(&(v1 as u32))),
        // Taken: 0 either way, 1 incomplete, 2 complete.
        9 => player.is_some_and(|p| {
            let Some(slot) = p.journal.slot_of(v1 as u32) else {
                return false;
            };
            let done = map
                .realm
                .data
                .npc
                .quests
                .get(&(v1 as u32))
                .is_some_and(|q| crate::game::map::quests::complete(p, q, &p.journal.log[slot]));
            match v2 {
                1 => !done,
                2 => done,
                _ => true,
            }
        }),
        22 => player.is_some_and(|p| {
            !p.journal.has(v1 as u32) && !p.journal.rewarded.contains(&(v1 as u32))
        }),
        19 => player.is_some_and(|p| {
            map.realm
                .data
                .npc
                .quests
                .get(&(v1 as u32))
                .is_some_and(|q| {
                    !p.journal.has(q.id)
                        && !p.journal.rewarded.contains(&q.id)
                        && crate::game::map::quests::can_take(p, q, &map.realm.data)
                })
        }),
        12 => map.realm.social.events.lock().contains(&(v1 as u32)),
        // Race and class masks.
        14 => player.is_some_and(|p| {
            let mask = |m: i32, v: u8| m == 0 || (m as u32) & (1 << (v.max(1) - 1)) != 0;
            mask(v1, p.row.race) && mask(v2, p.row.class)
        }),
        15 => unit_fields(target)
            .is_some_and(|f| compare(i64::from(f.get_u32(UNIT_FIELD_LEVEL)), i64::from(v1), v2)),
        // The source is one of these entries.
        16 => source.is_some_and(|s| {
            let entry = map
                .creature(s)
                .map(|c| c.entry)
                .or_else(|| map.gameobject(s).map(|o| o.entry))
                .unwrap_or(0) as i32;
            entry == v1
                || (v2 != 0 && entry == v2)
                || (v3 != 0 && entry == v3)
                || (v4 != 0 && entry == v4)
        }),
        17 => player.is_some_and(|p| {
            let known = p.spells.contains(&(v1 as u32));
            if v2 == 1 { !known } else { known }
        }),
        // A creature (alive, or with value 3, dead) near the source.
        20 => {
            let at = source.or(target).and_then(|g| map.position_of(g));
            at.is_some_and(|at| {
                !crate::game::map::scripts::creatures_near(map, at, v1 as u32, v2 as f32, v3 == 0)
                    .is_empty()
            })
        }
        21 => {
            let at = source.or(target).and_then(|g| map.position_of(g));
            at.is_some_and(|at| {
                map.gameobjects
                    .iter()
                    .any(|(_, o)| o.entry == v1 as u32 && dist(o.position, at) <= v2.max(1) as f32)
            })
        }
        23 => player.is_some_and(|p| p.count_of(v1 as u32) >= v2.max(1) as u32),
        24 => compare(
            i64::from(crate::game::data::PATCH as i32),
            i64::from(v1),
            v2,
        ),
        27 => {
            unit_fields(target).is_some_and(|f| i32::from(f.get_byte(UNIT_FIELD_BYTES_0, 2)) == v1)
        }
        28 => target.is_some_and(|t| {
            guid::is_player(t)
                || (v1 == 1
                    && map
                        .creature(t)
                        .is_some_and(|c| c.owner.is_some_and(guid::is_player)))
        }),
        // A field of the source has a flag.
        31 => source
            .and_then(|s| map.unit_fields(s))
            .is_some_and(|f| f.get_u32(v1 as u16) & v2 as u32 != 0),
        33 => map.key.map == v1 as u32,
        11 => compare(
            i64::from(crate::game::map::scripts::server_variable(v1 as u32)),
            i64::from(v2),
            v3,
        ),
        35 => crate::game::map::map_events::data(map, v1 as u32, v2 as u32)
            .is_some_and(|d| compare(i64::from(d), i64::from(v3), v4)),
        36 => crate::game::map::map_events::active(map, v1 as u32),
        47 => crate::game::map::map_events::targets(map, v1 as u32)
            .into_iter()
            .all(|t| eval(map, v2.max(0) as u32, source, Some(t), depth + 1)),
        34 => {
            let actual = crate::game::map::instance_script::get_data(map, v1 as u32);
            compare(i64::from(actual), i64::from(v2), v3)
        }
        // Line of sight between the two.
        37 => {
            source.and_then(|s| map.position_of(s)).is_some()
                && target.and_then(|t| map.position_of(t)).is_some()
        }
        38 => match (
            source.and_then(|s| map.position_of(s)),
            target.and_then(|t| map.position_of(t)),
        ) {
            (Some(a), Some(b)) => compare(dist(a, b) as i64, i64::from(v1), v2),
            _ => false,
        },
        39 => target.is_some_and(|t| {
            map.players
                .get(&t)
                .map(|p| p.movement.flags.get_forward())
                .or_else(|| {
                    map.creature(t)
                        .map(|c| matches!(c.motion, crate::game::map::creature::Motion::Moving(_)))
                })
                .unwrap_or(false)
        }),
        41 => unit_fields(target).is_some_and(|f| {
            compare(
                pct_of(
                    f.get_u32(UNIT_FIELD_HEALTH),
                    f.get_u32(UNIT_FIELD_MAXHEALTH),
                ),
                i64::from(v1),
                v2,
            )
        }),
        42 => unit_fields(target).is_some_and(|f| {
            compare(
                pct_of(
                    f.get_u32(UNIT_FIELD_POWER1),
                    f.get_u32(UNIT_FIELD_MAXPOWER1),
                ),
                i64::from(v1),
                v2,
            )
        }),
        43 => unit_fields(target).is_some_and(|f| {
            f.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
        }),
        45 => target.is_some_and(|t| map.realm.social.groups.lock().group_of(t).is_some()),
        46 => target.is_some_and(|t| map.unit_view(t).is_some_and(|u| u.alive)),
        48 => target.is_some_and(|t| map.gameobject(t).is_some()),
        51 => player.is_some_and(|p| {
            compare(
                i64::from(p.fields.get_byte(PLAYER_BYTES_3, 3)),
                i64::from(v1),
                v2,
            )
        }),
        // The source's spawn is one of these.
        52 => creature(source)
            .and_then(|c| c.spawn_guid)
            .is_some_and(|g| {
                let g = g as i32;
                g == v1 || (v2 != 0 && g == v2) || (v3 != 0 && g == v3) || (v4 != 0 && g == v4)
            }),
        53 => {
            use chrono::Timelike;
            let now = chrono::Local::now();
            let at = (now.hour() as i32, now.minute() as i32);
            at >= (v1, v2) && at <= (v3, v4)
        }
        55 => target
            .and_then(|t| map.gameobject(t))
            .is_some_and(|o| o.fields.get_u32(GAMEOBJECT_STATE) as i32 == v1),
        // A player near the source: any (0), hostile (1), friendly (2).
        56 => {
            let Some(at) = source.or(target).and_then(|g| map.position_of(g)) else {
                return false;
            };
            map.players.values().any(|p| {
                dist(p.position(), at) <= v1.max(1) as f32
                    && match v2 {
                        1 => source
                            .is_some_and(|s| crate::game::map::combat::hostile(map, s, p.guid)),
                        2 => source
                            .is_some_and(|s| !crate::game::map::combat::hostile(map, s, p.guid)),
                        _ => true,
                    }
            })
        }
        57 => source.is_some_and(|s| {
            let leader = crate::game::map::creature_groups::leader_of(map, s);
            leader.is_some() && (v1 == 0 || leader == Some(v1 as u32))
        }),
        58 => source.is_some_and(|s| crate::game::map::creature_groups::all_dead(map, s)),
        59 => player.is_some_and(|p| {
            map.realm.data.area(v1 as u32).is_some_and(|a| {
                let bit = a.explore_bit;
                let word = PLAYER_EXPLORED_ZONES_1 + (bit / 32) as u16;
                p.fields.get_u32(word) & (1 << (bit % 32)) != 0
            })
        }),
        // Saved variables, escorts, holidays, map events and the like
        // aren't tracked.
        _ => false,
    }
}
