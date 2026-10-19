//! EventAI: a creature's `creature_ai_events`, as vmangos's
//! `CreatureEventAI` runs them and Thistle Tea ports it. Timed events
//! (timers in and out of combat, health and mana thresholds, range, friendly
//! health, auras) are checked about every half second while the creature is
//! active; edge events (aggro, death, evade, kill, spell hit, reached home,
//! emotes, script events, summons) fire when they happen. A firing event's
//! actions run as `creature_ai_scripts` through [`super::scripts`].
//!
//! Phases follow vmangos: an event is skipped in the phases its inverse
//! phase mask names. Non-repeatable events fire once a combat; timers
//! re-roll from their repeat parameters.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::scripts::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::scripts;

/// How often timed events are looked at (Thistle Tea's second).
const TICK: Duration = Duration::from_secs(1);
const FRIENDLY_RADIUS: f32 = 30.0;

/// What scripts keep on a creature: its EventAI state, and what scripts
/// have set (phase, movement and melee, invincibility, a summoner, a
/// despawn timer).
#[derive(Debug, Clone, Default)]
pub struct Scripted {
    pub phase: u32,
    /// Each event's timer: due when passed; `None` due at once.
    pub timers: Vec<Option<Instant>>,
    pub disabled: Vec<bool>,
    pub started: bool,
    pub next_tick: Option<Instant>,
    /// When its timers were last brought up to date (Thistle Tea's
    /// `last_timer_update_at`): a stun's pause adds what passed since. A
    /// fight's start and an evade set it again, so the time between
    /// fights isn't counted as stunned.
    pub clock: Option<Instant>,
    pub no_melee: bool,
    pub no_combat_movement: bool,
    /// A script turned melee, or combat movement, on: over its template's
    /// default (a no-melee or sessile creature's).
    pub melee_on: bool,
    pub combat_movement_on: bool,
    /// Health it can't be brought below, as a script set it (None: its
    /// template's, a floor of 1 for one that is unkillable).
    pub invincible_hp: Option<u32>,
    pub summoner: Option<Guid>,
    /// When a summoned creature goes, and how (vmangos's despawn types,
    /// as Thistle Tea's `SummonDespawn` runs them), and the time it was
    /// summoned for, which a fight's end or its death may start again.
    pub despawn_at: Option<Instant>,
    pub despawn_type: i32,
    pub despawn_after: Option<Duration>,
    /// A called guard's Avoidance, in percent: put back after an evade.
    pub guard_avoidance: Option<i32>,
    /// The entry whose events it runs: the one it had before a script
    /// changed it (`update_entry`), as vmangos and Thistle Tea keep them.
    pub ai_entry: Option<u32>,
    /// The spell list it keeps over its template's, from an entry it was
    /// before that had one where its new one has none.
    pub base_spell_list: Option<u32>,
    /// A guardian a creature's spell summoned (effect 42): it follows
    /// its summoner, goes when it goes, and drops no loot.
    pub guardian: bool,
    pub summoned: Vec<Guid>,
    /// Script moves run rather than walk.
    pub run: bool,
    /// Passive: it doesn't look for enemies.
    pub passive: bool,
    /// A reaction mode a script set, over its template's.
    pub reaction: Option<crate::game::map::assistance::Reaction>,
    /// A spell list a script set, over its template's.
    pub spell_list: Option<u32>,
    /// A gossip menu a script set.
    pub gossip_menu: Option<u32>,
    /// It was in combat at the last tick.
    pub in_combat: bool,
    /// A script's move under way.
    pub scripted_move: Option<ScriptedMove>,
    /// It flies (`SCRIPT_COMMAND_SET_FLY`).
    pub fly: bool,
    /// A unit it follows: (whom, distance, angle from its facing).
    pub follow: Option<(Guid, f32, f32)>,
    /// Hidden from players until a script shows it.
    pub concealed: bool,
    /// A triggered spell its swings are, in place of plain blows.
    pub melee_spell: Option<u32>,
    /// How far it calls for help on aggro (0 not at all), over its
    /// template's.
    pub call_for_help_range: Option<f32>,
    /// A caster that stops this far from its victim, not in melee.
    pub caster_chase_distance: Option<f32>,
    /// What it regenerates out of combat (Thistle Tea's stats mask), over
    /// its own.
    pub regenerate: Option<u32>,
    /// Its path held where it is.
    pub hold: Option<Hold>,
    /// It feigns death at a killing blow while kin of its are near.
    pub feign: Option<Feign>,
    /// A path a script set it walking, over its spawn's.
    pub route: Option<Route>,
    /// Where a script set it wandering about (movement 1 with datalong2),
    /// over its home, which stays its spawn's (Thistle Tea's runtime
    /// random anchor).
    pub wander_anchor: Option<Vector3d>,
    /// A faction a script set (`SCRIPT_COMMAND_SET_FACTION`): the one it
    /// had, the one set, and when the old one comes back (datalong2).
    pub temp_faction: Option<TempFaction>,
    /// A despawn a script put off (`SCRIPT_COMMAND_DESPAWN_SELF`): when,
    /// and the respawn's delay over its spawn's.
    pub script_despawn: Option<(Instant, Option<Duration>)>,
}

/// A script's faction for a while, as Thistle Tea's `TemporaryFaction`
/// keeps it: the old one back at a respawn (0x1), when a fight stops
/// (0x2) or home again (0x4), as its flags say; kept over a respawn
/// without 0x1.
#[derive(Debug, Clone, Copy)]
pub struct TempFaction {
    pub original: u32,
    pub value: u32,
    pub flags: u32,
}

pub const FACTION_RESTORE_RESPAWN: u32 = 0x1;
pub const FACTION_RESTORE_COMBAT_STOP: u32 = 0x2;
pub const FACTION_RESTORE_REACH_HOME: u32 = 0x4;

/// A path held, as vmangos's escorts pause: until the summons made since
/// are gone (or, `signal`, until a script lets it go), and at most until
/// `until`. Its release runs a script; one that ran out unsignalled runs
/// `expired` if it has one.
#[derive(Debug, Clone)]
pub struct Hold {
    pub until: Instant,
    pub ready_at: Instant,
    pub signal: bool,
    pub signaled: bool,
    pub earlier: Vec<Guid>,
    pub release_script: u32,
    pub expired_script: u32,
}

/// Linked feign death: a killing blow leaves it down, not dead, and after
/// `delay` it gets up unless every creature of `entry` within `radius`
/// is down too. `fall` and `revive` are scripts; `pacify` the aura it
/// lies under.
#[derive(Debug, Clone, Copy)]
pub struct Feign {
    pub delay: Duration,
    pub entry: u32,
    pub radius: f32,
    pub pacify: u32,
    pub fall_script: u32,
    pub revive_script: u32,
    /// Down since then.
    pub down_at: Option<Instant>,
}

/// A path a script started: its points, and whether it goes round again.
#[derive(Debug, Clone)]
pub struct Route {
    pub points: std::sync::Arc<Vec<crate::game::data::Waypoint>>,
    pub repeat: bool,
}

/// A move a script started: its spline, where it ends, the point its
/// arrival tells of (a movement inform of type 9), and which way it faces
/// there.
#[derive(Debug, Clone, Copy)]
pub struct ScriptedMove {
    pub spline_id: u32,
    pub to: wow_world_messages::vanilla::Vector3d,
    pub point: Option<u32>,
    pub face: Option<f32>,
    /// A walk home (`SCRIPT_COMMAND_MOVEMENT` 7): home, it is as an
    /// evade's arrival.
    pub home: bool,
    /// Distancing (movement 19): only force-cast spells go meanwhile.
    pub distancing: Option<Retreat>,
}

/// A retreat from a unit (movement 19): whom it backs from, and the
/// victim and fight it had when it began.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Retreat {
    pub from: Guid,
    pub victim: Guid,
    pub fighting: bool,
}

/// Whether a creature may back away (Thistle Tea's `Distancing.allowed?`):
/// not stunned, feared, confused or rooted, not running from a fight, not
/// feigning death.
pub fn may_retreat(map: &Map, g: Guid) -> bool {
    use crate::game::map::auras::{AURA_FEIGN_DEATH, auras, held};
    !held(map, g, false)
        && !crate::game::map::creature_combat::immobile(map, g)
        && !auras(map, g).is_some_and(|a| a.has(AURA_FEIGN_DEATH))
        && map.creature(g).is_some_and(|c| c.engagement.flee.is_none())
}

/// A retreat under way is called off, the creature stopping where it is,
/// once it may no longer back away, its fight began or ended, its victim
/// changed, or the one it backs from is dead or gone (Thistle Tea's
/// `Distancing.maintain`). A walk that took the retreat's place goes on.
pub fn retreat_step(map: &mut Map, g: Guid) {
    let Some(c) = map.creature(g) else {
        return;
    };
    let Some((m, r)) = c
        .scripted
        .as_deref()
        .and_then(|s| s.scripted_move)
        .and_then(|m| m.distancing.map(|r| (m, r)))
    else {
        return;
    };
    let own = matches!(&c.motion, crate::game::map::creature::Motion::Moving(w) if w.spline_id == m.spline_id);
    if !own {
        return;
    }
    let off = c.combat.attacking != r.fighting
        || c.combat.victim != r.victim
        || map.unit_view(r.from).is_none_or(|u| !u.alive)
        || !may_retreat(map, g);
    if !off {
        return;
    }
    crate::game::map::ai::halt(map, g);
    if let Some(s) = map.creature_mut(g).and_then(|c| c.scripted.as_deref_mut()) {
        s.scripted_move = None;
    }
}

/// The motion type of a script's point movement.
pub const POINT_MOTION: u32 = 9;

/// The entry whose events a creature runs.
pub fn ai_entry(c: &crate::game::map::creature::Creature) -> u32 {
    c.scripted
        .as_deref()
        .and_then(|s| s.ai_entry)
        .unwrap_or(c.entry)
}

/// The events of creature `entry`.
fn events_of(map: &Map, entry: u32) -> Option<&[AiEvent]> {
    map.realm
        .data
        .scripts
        .ai_events
        .get(&entry)
        .map(Vec::as_slice)
        .filter(|e| !e.is_empty())
}

pub fn has_events_entry(data: &crate::game::data::GameData, entry: u32) -> bool {
    data.scripts
        .ai_events
        .get(&entry)
        .is_some_and(|e| !e.is_empty())
}

pub fn has_events(map: &Map, g: Guid) -> bool {
    map.creature(g)
        .is_some_and(|c| events_of(map, ai_entry(c)).is_some())
}

/// The creature's scripted state, made if it has none.
pub fn scripted_mut(map: &mut Map, g: Guid) -> Option<&mut Scripted> {
    let c = map.creature_mut(g)?;
    Some(c.scripted.get_or_insert_with(Default::default))
}

pub fn scripted(map: &Map, g: Guid) -> Option<&Scripted> {
    map.creature(g).and_then(|c| c.scripted.as_deref())
}

fn roll(lo: i32, hi: i32) -> u64 {
    let (lo, hi) = (lo.max(0) as u64, hi.max(0) as u64);
    if hi > lo {
        rand::rng().random_range(lo..=hi)
    } else {
        lo
    }
}

fn in_combat(map: &Map, g: Guid) -> bool {
    map.creature(g).is_some_and(|c| {
        c.combat.attacking
            || c.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
    })
}

fn victim(map: &Map, g: Guid) -> Option<Guid> {
    map.creature(g)
        .map(|c| c.combat.victim)
        .filter(|v| !v.is_zero() && in_combat(map, g))
}

/// Timers and enables set up the first time, and out-of-combat timers
/// rolled.
fn ensure_started(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(entry) = map.creature(g).map(ai_entry) else {
        return;
    };
    let Some(events) = events_of(map, entry).map(<[AiEvent]>::to_vec) else {
        return;
    };
    let Some(s) = scripted_mut(map, g) else {
        return;
    };
    if s.started && s.timers.len() == events.len() {
        return;
    }
    s.started = true;
    s.timers = vec![None; events.len()];
    s.disabled = vec![false; events.len()];
    reset_ooc(s, &events, now);
}

fn reset_ooc(s: &mut Scripted, events: &[AiEvent], now: Instant) {
    s.clock = Some(now);
    for (i, e) in events.iter().enumerate() {
        if e.kind == EVENT_TIMER_OOC {
            s.timers[i] = Some(now + Duration::from_millis(roll(e.params[0], e.params[1])));
            s.disabled[i] = false;
        }
    }
}

fn reset_for_combat(s: &mut Scripted, events: &[AiEvent], now: Instant) {
    s.clock = Some(now);
    s.timers = vec![None; events.len()];
    s.disabled = vec![false; events.len()];
    for (i, e) in events.iter().enumerate() {
        if e.kind == EVENT_TIMER_IN_COMBAT {
            s.timers[i] = Some(now + Duration::from_millis(roll(e.params[0], e.params[1])));
        }
    }
}

/// The repeat timer's range for an event that fired.
fn repeat_params(e: &AiEvent) -> Option<(i32, i32)> {
    match e.kind {
        EVENT_KILL | EVENT_VICTIM_ROOTED => Some((e.params[0], e.params[1])),
        EVENT_SUMMONED_UNIT | EVENT_SUMMONED_JUST_DIED | EVENT_SUMMONED_JUST_DESPAWN => {
            Some((e.params[1], e.params[2]))
        }
        EVENT_TIMER_IN_COMBAT
        | EVENT_TIMER_OOC
        | EVENT_HP
        | EVENT_MANA
        | EVENT_TARGET_HP
        | EVENT_TARGET_MANA
        | EVENT_RANGE
        | EVENT_OOC_LOS
        | EVENT_FRIENDLY_HP
        | EVENT_FRIENDLY_IS_CC
        | EVENT_FRIENDLY_MISSING_BUFF
        | EVENT_HIT_BY_SPELL
        | EVENT_SPELL_HIT_TARGET
        | EVENT_MOVEMENT_INFORM
        | EVENT_AURA
        | EVENT_TARGET_AURA
        | EVENT_MISSING_AURA
        | EVENT_TARGET_MISSING_AURA => Some((e.params[2], e.params[3])),
        _ => None,
    }
}

fn timed(kind: u32) -> bool {
    matches!(
        kind,
        EVENT_TIMER_IN_COMBAT
            | EVENT_TIMER_OOC
            | EVENT_HP
            | EVENT_MANA
            | EVENT_RANGE
            | EVENT_OOC_LOS
            | EVENT_TARGET_HP
            | EVENT_FRIENDLY_HP
            | EVENT_FRIENDLY_IS_CC
            | EVENT_FRIENDLY_MISSING_BUFF
            | EVENT_TARGET_MANA
            | EVENT_AURA
            | EVENT_TARGET_AURA
            | EVENT_MISSING_AURA
            | EVENT_TARGET_MISSING_AURA
            | EVENT_VICTIM_ROOTED
    )
}

/// Health as a whole percentage, rounded down, as Thistle Tea has it.
fn health_pct(map: &Map, g: Guid) -> Option<f32> {
    let f = map.unit_fields(g)?;
    let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
    if max == 0 {
        return Some(100.0);
    }
    Some((f.get_u32(UNIT_FIELD_HEALTH) as f32 / max as f32 * 100.0).trunc())
}

/// Mana likewise; none for one with no mana.
fn mana_pct(map: &Map, g: Guid) -> Option<f32> {
    let f = map.unit_fields(g)?;
    let max = f.get_u32(UNIT_FIELD_MAXPOWER1);
    if max == 0 {
        return None;
    }
    Some((f.get_u32(UNIT_FIELD_POWER1) as f32 / max as f32 * 100.0).trunc())
}

/// The most stacks of `spell` any one holder has on `g`.
fn stacks_of(map: &Map, g: Guid, spell: u32) -> u32 {
    crate::game::map::auras::auras(map, g)
        .and_then(|a| {
            a.holders
                .iter()
                .filter(|h| h.spell == spell)
                .map(|h| h.stacks.max(1))
                .max()
        })
        .unwrap_or(0)
}

fn within(pct: f32, e: &AiEvent) -> bool {
    pct <= e.params[0] as f32 && pct >= e.params[1] as f32
}

/// Whether an event's own test passes, and whom it fires at.
fn satisfy(map: &Map, g: Guid, e: &AiEvent, invoker: Option<Guid>) -> Option<Option<Guid>> {
    let combat = in_combat(map, g);
    let ok = |b: bool| b.then_some(invoker);
    match e.kind {
        EVENT_TIMER_IN_COMBAT => ok(combat),
        EVENT_TIMER_OOC => ok(!combat),
        EVENT_HP => ok(combat && health_pct(map, g).is_some_and(|p| within(p, e))),
        EVENT_MANA => ok(combat && mana_pct(map, g).is_some_and(|p| within(p, e))),
        EVENT_TARGET_HP => {
            let v = victim(map, g)?;
            ok(health_pct(map, v).is_some_and(|p| within(p, e)))
        }
        EVENT_TARGET_MANA => {
            let v = victim(map, g)?;
            ok(mana_pct(map, v).is_some_and(|p| within(p, e)))
        }
        EVENT_RANGE => {
            let v = victim(map, g)?;
            let (a, b) = (map.position_of(g)?, map.position_of(v)?);
            let d = dist(a, b);
            ok(d >= e.params[0] as f32 && d <= e.params[1] as f32)
        }
        // Its own look about, with line of sight: `ooc_los`.
        EVENT_OOC_LOS => None,
        // The most hurt of itself and its friends about, in a fight, by
        // the share of health missing (over 1%), as Thistle Tea picks them:
        // its first parameter, a health deficit, plays no part.
        EVENT_FRIENDLY_HP => {
            if !combat {
                return None;
            }
            let radius = if e.params[1] > 0 {
                e.params[1] as f32
            } else {
                FRIENDLY_RADIUS
            };
            let missing = |u: Guid| 100.0 - health_pct(map, u).unwrap_or(100.0);
            let mut best: Option<(Guid, f32)> = None;
            let alive = map.unit_view(g).is_some_and(|u| u.alive);
            let candidates = (alive && missing(g) > 1.0).then_some(g).into_iter().chain(
                nearby(map, g, radius).into_iter().filter(|u| {
                    in_fight_and_selectable(map, *u)
                        && crate::game::map::combat::friendly(map, g, *u)
                        && missing(*u) > 1.0
                }),
            );
            for u in candidates {
                let m = missing(u);
                if best.is_none_or(|(_, b)| m > b) {
                    best = Some((u, m));
                }
            }
            Some(Some(best?.0))
        }
        EVENT_FRIENDLY_IS_CC => {
            if !combat {
                return None;
            }
            let radius = if e.params[1] > 0 {
                e.params[1] as f32
            } else {
                FRIENDLY_RADIUS
            };
            let found = friendlies(map, g, radius)
                .into_iter()
                .find(|f| crate::game::map::auras::held(map, *f, false))?;
            Some(Some(found))
        }
        EVENT_FRIENDLY_MISSING_BUFF => {
            let radius = if e.params[1] > 0 {
                e.params[1] as f32
            } else {
                FRIENDLY_RADIUS
            };
            let spell = e.params[0].max(0) as u32;
            let found = friendlies(map, g, radius)
                .into_iter()
                .find(|f| stacks_of(map, *f, spell) == 0)?;
            Some(Some(found))
        }
        EVENT_AURA => {
            ok(combat && stacks_of(map, g, e.params[0].max(0) as u32) >= e.params[1].max(0) as u32)
        }
        EVENT_MISSING_AURA => {
            ok(combat && stacks_of(map, g, e.params[0].max(0) as u32) < e.params[1].max(0) as u32)
        }
        EVENT_TARGET_AURA => {
            let v = victim(map, g)?;
            ok(combat && stacks_of(map, v, e.params[0].max(0) as u32) >= e.params[1].max(0) as u32)
        }
        EVENT_TARGET_MISSING_AURA => {
            let v = victim(map, g)?;
            ok(combat && stacks_of(map, v, e.params[0].max(0) as u32) < e.params[1].max(0) as u32)
        }
        EVENT_VICTIM_ROOTED => {
            let v = victim(map, g)?;
            ok(crate::game::map::auras::auras(map, v)
                .is_some_and(|a| a.has(crate::game::map::auras::AURA_MOD_ROOT)))
        }
        EVENT_KILL => {
            if e.params[2] == 1 && !invoker.is_some_and(guid::is_player) {
                None
            } else {
                Some(invoker)
            }
        }
        EVENT_AGGRO
        | EVENT_SPAWNED
        | EVENT_DEATH
        | EVENT_EVADE
        | EVENT_LEAVE_COMBAT
        | EVENT_HIT_BY_SPELL
        | EVENT_REACHED_HOME
        | EVENT_RECEIVE_EMOTE
        | EVENT_SPELL_HIT_TARGET
        | EVENT_SCRIPT_EVENT
        | EVENT_GROUP_MEMBER_DIED
        | EVENT_SUMMONED_UNIT
        | EVENT_SUMMONED_JUST_DIED
        | EVENT_SUMMONED_JUST_DESPAWN
        | EVENT_MOVEMENT_INFORM => Some(invoker),
        _ => None,
    }
}

/// The units near `g`, creatures then players, each nearest first.
fn nearby(map: &Map, g: Guid, radius: f32) -> Vec<Guid> {
    let Some(me) = map.position_of(g) else {
        return Vec::new();
    };
    let mut out: Vec<(bool, f32, Guid)> = map
        .grid
        .around(me.x, me.y, radius)
        .filter(|u| *u != g && (guid::is_unit(*u) || guid::is_player(*u)))
        .filter_map(|u| {
            let d = dist(map.position_of(u)?, me);
            (d <= radius).then_some((guid::is_player(u), d, u))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    out.into_iter().map(|(_, _, u)| u).collect()
}

/// Alive, in a fight and selectable.
fn in_fight_and_selectable(map: &Map, u: Guid) -> bool {
    map.unit_view(u).is_some_and(|v| v.alive)
        && map.unit_fields(u).is_some_and(|f| {
            let flags = f.get_u32(UNIT_FIELD_FLAGS);
            flags & crate::game::map::player::UNIT_FLAG_IN_COMBAT != 0 && flags & 0x0200_0000 == 0
        })
}

/// The units a friendly event looks at, itself first, then those near:
/// alive, in a fight, selectable and not hostile to it (Thistle Tea's
/// `friendly_candidates`).
fn friendlies(map: &Map, g: Guid, radius: f32) -> Vec<Guid> {
    std::iter::once(g)
        .chain(nearby(map, g, radius))
        .filter(|u| {
            in_fight_and_selectable(map, *u)
                && (*u == g || !crate::game::map::combat::hostile(map, g, *u))
        })
        .collect()
}

/// An out-of-combat line-of-sight event's unit: the nearest within its
/// radius of the reaction it names (any, hostile or not), in sight, that
/// meets its condition.
fn ooc_los(map: &mut Map, g: Guid, e: &AiEvent) -> Option<Option<Guid>> {
    if in_combat(map, g) {
        return None;
    }
    let radius = if e.params[1] > 0 {
        e.params[1] as f32
    } else {
        FRIENDLY_RADIUS
    };
    let me = map.position_of(g)?;
    let mut near: Vec<(f32, Guid)> = map
        .grid
        .around(me.x, me.y, radius)
        .filter(|u| *u != g && (guid::is_player(*u) || guid::is_unit(*u)))
        .filter_map(|u| {
            let v = map.unit_view(u)?;
            let d = dist(v.position, me);
            (v.alive && d <= radius).then_some((d, u))
        })
        .filter(|(_, u)| !map.players.get(u).is_some_and(|p| p.gm))
        .filter(|(_, u)| match e.params[0] {
            0 => true,
            1 => crate::game::map::combat::hostile(map, g, *u),
            2 => !crate::game::map::combat::hostile(map, g, *u),
            _ => false,
        })
        .filter(|(_, u)| {
            e.condition == 0
                || crate::game::map::conditions::met(map, e.condition, Some(g), Some(*u))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let found = near.into_iter().map(|(_, u)| u).find(|u| {
        map.position_of(*u)
            .is_some_and(|to| crate::game::map::assistance::sees(map, me, to))
    })?;
    Some(Some(found))
}

/// Try the events `matches` picks, with `invoker` as the one who caused
/// them.
fn fire(map: &mut Map, g: Guid, matches: impl Fn(&AiEvent) -> bool, invoker: Option<Guid>) {
    let Some(entry) = map.creature(g).filter(|c| c.owner.is_none()).map(ai_entry) else {
        return;
    };
    let Some(events) = events_of(map, entry).map(<[AiEvent]>::to_vec) else {
        return;
    };
    ensure_started(map, g);
    let now = map.now;
    for (i, e) in events.iter().enumerate() {
        if !matches(e) || !event_supported(e.kind) {
            continue;
        }
        let Some(s) = scripted(map, g) else {
            return;
        };
        if s.disabled.get(i).copied().unwrap_or(false)
            || s.timers.get(i).copied().flatten().is_some_and(|t| t > now)
            || e.inverse_phase_mask & (1 << s.phase.min(31)) != 0
        {
            continue;
        }
        if e.flags & EF_NOT_CASTING != 0
            && map.creature(g).is_some_and(|c| c.caster.casting.is_some())
        {
            continue;
        }
        if e.flags & EF_PAUSE_WHILE_STUNNED != 0
            && map
                .creature(g)
                .is_some_and(|c| c.auras.has(crate::game::map::auras::AURA_MOD_STUN))
        {
            continue;
        }
        if e.flags & EF_VICTIM_IN_MELEE != 0
            && !victim(map, g).is_some_and(|v| {
                matches!((map.unit_view(g), map.unit_view(v)), (Some(a), Some(b))
                    if crate::game::map::combat::in_melee_range(&a, &b))
            })
        {
            continue;
        }
        let target = if e.kind == EVENT_OOC_LOS {
            ooc_los(map, g, e)
        } else {
            satisfy(map, g, e, invoker)
        };
        let Some(target) = target else {
            continue;
        };
        if e.kind != EVENT_OOC_LOS
            && e.condition != 0
            && !crate::game::map::conditions::met(
                map,
                e.condition,
                Some(g),
                target.or_else(|| victim(map, g)).or(Some(g)),
            )
        {
            continue;
        }
        if let Some(s) = scripted_mut(map, g) {
            match repeat_params(e) {
                Some((lo, hi)) if hi >= lo => {
                    s.timers[i] = Some(now + Duration::from_millis(roll(lo, hi)));
                }
                Some(_) => s.disabled[i] = true,
                None => {}
            }
            if e.flags & EF_REPEATABLE == 0 {
                s.disabled[i] = true;
            }
        }
        if e.chance < 100 && e.chance > 0 && rand::rng().random_range(0..100) >= e.chance {
            continue;
        }
        if e.chance == 0 {
            continue;
        }
        run_actions(map, g, i, e, target.or_else(|| victim(map, g)));
        // An action may have killed or despawned the creature.
        if map.creature(g).is_none() {
            return;
        }
    }
}

fn run_actions(map: &mut Map, g: Guid, i: usize, e: &AiEvent, target: Option<Guid>) {
    // An action whose script is missing is none (Thistle Tea drops it as
    // it loads), so a random pick never falls on it.
    let scripts = &map.realm.data.scripts.ai_scripts;
    let actions: Vec<u32> = e
        .actions
        .iter()
        .copied()
        .filter(|a| *a != 0 && scripts.contains_key(a))
        .collect();
    let chosen: Vec<u32> = if e.flags & EF_RANDOM_ACTION != 0 && !actions.is_empty() {
        vec![actions[rand::rng().random_range(0..actions.len())]]
    } else {
        actions
    };
    let checked = (e.flags & EF_CHECK_RESULT != 0).then_some((g, i));
    for script in chosen {
        scripts::start_checked(
            map,
            ScriptTable::CreatureAi,
            script,
            g,
            target.unwrap_or_default(),
            checked,
        );
    }
}

/// A result-checked event whose actions failed: due again at once, as a
/// vmangos timer starts over only when its cast lands.
pub fn retry(map: &mut Map, g: Guid, i: usize) {
    let now = map.now;
    if let Some(s) = map.creature_mut(g).and_then(|c| c.scripted.as_deref_mut())
        && i < s.timers.len()
    {
        s.timers[i] = Some(now);
        s.disabled[i] = false;
    }
}

/// A script's move ended: its facing, and the point it tells of.
pub fn scripted_move_ended(map: &mut Map, g: Guid) {
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let Some(m) = c.scripted.as_deref().and_then(|s| s.scripted_move) else {
        return;
    };
    let moving = match &c.motion {
        crate::game::map::creature::Motion::Moving(w) => Some(w.spline_id),
        _ => None,
    };
    if moving == Some(m.spline_id) {
        return;
    }
    if let Some(s) = c.scripted.as_deref_mut() {
        s.scripted_move = None;
    }
    // Another move took its place, or it stopped short.
    if moving.is_some() || dist(c.position, m.to) > 2.0 {
        return;
    }
    // Its distance kept, its spell list tries again at once (Thistle
    // Tea's `Distancing.tick` releasing the list's deadline).
    if m.distancing.is_some()
        && let Some(l) = c.caster.spell_list.as_deref_mut()
    {
        l.next_at = None;
    }
    if let Some(o) = m.face {
        c.orientation = o;
        map.visibility_dirty.insert(g);
    }
    if let Some(point) = m.point {
        movement_inform(map, g, POINT_MOTION, point);
    }
    if m.home {
        home_reached(map, g);
    }
}

/// A script's walk home done, as Thistle Tea's `wait_for_scripted_home`:
/// it moves as its spawn does again (a script's path, wander or stillness
/// over), a faction set until then gives way, its spawn's auras are back,
/// and its reached-home events fire.
pub fn home_reached(map: &mut Map, g: Guid) {
    if let Some((kind, distance)) = spawn_movement(map, g) {
        let now = map.now;
        if let Some(c) = map.creature_mut(g) {
            c.movement_type = kind;
            c.wander_distance = distance;
            c.motion = crate::game::map::creature::Motion::Idle { until: Some(now) };
            if let Some(s) = c.scripted.as_deref_mut() {
                s.route = None;
                s.wander_anchor = None;
            }
        }
    }
    scripts::restore_faction(map, g, FACTION_RESTORE_REACH_HOME);
    crate::game::map::spawn_auras::restore(map, g);
    reached_home(map, g);
}

/// How a creature's spawn moves: its movement type (a path of its own
/// walked, a wander of no distance none) and its wander distance, as it
/// came into the world.
fn spawn_movement(map: &Map, g: Guid) -> Option<(u8, f32)> {
    use crate::game::map::creature::{MOVEMENT_IDLE, MOVEMENT_RANDOM, MOVEMENT_WAYPOINT};
    let data = &map.realm.data;
    let spawn = map.creature(g)?.spawn_guid?;
    let s = data
        .creature_spawns
        .get(&map.key.map)?
        .iter()
        .find(|s| s.guid == spawn)?;
    let path = data.waypoints.get(&spawn).is_some_and(|p| !p.is_empty());
    let kind = match s.movement_type {
        k if path && k != crate::game::map::creature_motion::MOVEMENT_CYCLIC => MOVEMENT_WAYPOINT,
        MOVEMENT_WAYPOINT if !path => MOVEMENT_IDLE,
        MOVEMENT_RANDOM if s.wander_distance <= 0.0 => MOVEMENT_IDLE,
        k => k,
    };
    Some((kind, s.wander_distance))
}

/// Where a wandering creature wanders about: a script's anchor, else its
/// home.
pub fn wander_center(c: &crate::game::map::creature::Creature) -> Vector3d {
    c.scripted
        .as_deref()
        .and_then(|s| s.wander_anchor)
        .unwrap_or(c.home)
}

/// The timed events of an active creature, about twice a second.
pub fn tick(map: &mut Map, g: Guid) {
    let now = map.now;
    if !has_events(map, g) {
        return;
    }
    let combat = in_combat(map, g);
    let Some(s) = scripted_mut(map, g) else {
        return;
    };
    if s.next_tick.is_some_and(|t| t > now) {
        return;
    }
    // The time since its timers were last brought up to date, which a
    // stun's pause adds back.
    let since = s
        .clock
        .map_or(Duration::ZERO, |last| now.saturating_duration_since(last));
    s.clock = Some(now);
    s.next_tick = Some(now + TICK);
    // A stunned creature's paused timers wait out the stun.
    let paused: Vec<usize> = map
        .creature(g)
        .and_then(|c| events_of(map, ai_entry(c)))
        .map(|events| {
            events
                .iter()
                .enumerate()
                .filter(|(_, e)| e.flags & EF_PAUSE_WHILE_STUNNED != 0)
                .map(|(i, _)| i)
                .collect()
        })
        .unwrap_or_default();
    let stunned = map
        .creature(g)
        .is_some_and(|c| c.auras.has(crate::game::map::auras::AURA_MOD_STUN));
    if !paused.is_empty()
        && stunned
        && let Some(s) = scripted_mut(map, g)
    {
        for i in paused {
            if let Some(Some(t)) = s.timers.get_mut(i) {
                *t += since;
            }
        }
    }
    let Some(s) = scripted_mut(map, g) else {
        return;
    };
    let was = s.in_combat;
    s.in_combat = combat;
    if was && !combat {
        fire(map, g, |e| e.kind == EVENT_LEAVE_COMBAT, None);
    }
    fire(map, g, |e| timed(e.kind), None);
}

// vmangos's `TempSummonType`s, as Thistle Tea's `SummonDespawn` runs them:
// which time from the summoning, run on after a death, wait out a fight
// (and start again after it), vanish at death, or time from it.
const TIMED_WHILE_ALIVE: [i32; 6] = [1, 2, 3, 4, 9, 10];
const TIMED_WHILE_DEAD: [i32; 5] = [3, 4, 6, 9, 10];
const WAITS_OUT_COMBAT: [i32; 3] = [1, 2, 4];
const GONE_AT_DEATH: [i32; 3] = [2, 5, 10];
const TIMER_FROM_DEATH: [i32; 2] = [4, 6];
/// A summon whose time came in a fight looks again this much later.
const DESPAWN_RECHECK: Duration = Duration::from_secs(10);
/// A wild summon whose death came in a fight looks again this much later.
const TIMED_DEATH_RECHECK: Duration = Duration::from_millis(100);
/// Thistle Tea's own type for a creature's wild summon (spell effect 41)
/// with a duration: at its time, out of a fight, it dies (`TemporarySummon`),
/// its corpse left as any's.
pub const TIMED_DEATH: i32 = 11;

/// What a summon whose time has come does.
enum Due {
    Despawn,
    Die,
    Wait,
    Ignore,
}

fn when_due(kind: i32, alive: bool, fighting: bool) -> Due {
    if kind == TIMED_DEATH {
        if !alive {
            Due::Ignore
        } else if fighting {
            Due::Wait
        } else {
            Due::Die
        }
    } else if kind <= 0 {
        Due::Despawn
    } else if !alive {
        if TIMED_WHILE_DEAD.contains(&kind) {
            Due::Despawn
        } else {
            Due::Ignore
        }
    } else if !TIMED_WHILE_ALIVE.contains(&kind) {
        Due::Ignore
    } else if fighting && WAITS_OUT_COMBAT.contains(&kind) {
        Due::Wait
    } else {
        Due::Despawn
    }
}

/// Whether a summon of despawn type `kind` starts its time when it is
/// summoned (the rest from their death, or never).
pub fn timer_at_spawn(kind: i32) -> bool {
    kind <= 0 || TIMED_WHILE_ALIVE.contains(&kind)
}

/// A summon died: one of a type that vanishes at death goes (the summoner
/// told of that too), one that times from its death starts its time.
pub fn summon_died(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some((kind, after)) = map
        .creature(g)
        .filter(|c| c.spawn_guid.is_none())
        .and_then(|c| c.scripted.as_deref())
        .map(|s| (s.despawn_type, s.despawn_after))
    else {
        return;
    };
    if GONE_AT_DEATH.contains(&kind) {
        scripts::despawn(map, g);
    } else if TIMER_FROM_DEATH.contains(&kind)
        && let Some(after) = after
        && let Some(s) = scripted_mut(map, g)
    {
        s.despawn_at = Some(now + after);
    }
}

/// A creature's despawn timer, for those summoned with one, and the
/// despawns scripts put off.
pub fn update_despawns(map: &mut Map) {
    let now = map.now;
    let z = crate::zone!("scripts' despawns");
    let put_off: Vec<(Guid, Option<Duration>)> = map
        .creatures
        .iter()
        .filter_map(|(_, c)| {
            let (at, respawn) = c.scripted.as_deref()?.script_despawn?;
            (at <= now).then_some((c.guid, respawn))
        })
        .collect();
    for (g, respawn) in put_off {
        if let Some(s) = scripted_mut(map, g) {
            s.script_despawn = None;
        }
        scripts::despawn_with(map, g, respawn);
    }
    drop(z);
    let z = crate::zone!("orphaned guardians");
    // A creature's guardians go with it, as Thistle Tea's `owner_stopped`.
    let orphans: Vec<Guid> = map
        .creatures
        .iter()
        .filter_map(|(_, c)| {
            let s = c.scripted.as_deref()?;
            (s.guardian && s.summoner.is_some_and(|o| map.creature(o).is_none())).then_some(c.guid)
        })
        .collect();
    for g in orphans {
        scripts::despawn(map, g);
    }
    drop(z);
    let _z = crate::zone!("despawn timers");
    let due: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.scripted
                .as_deref()
                .and_then(|s| s.despawn_at)
                .is_some_and(|t| t <= now)
        })
        .map(|(_, c)| c.guid)
        .collect();
    for g in due {
        let alive = map.creature(g).is_some_and(|c| c.alive());
        let kind = scripted(map, g).map_or(0, |s| s.despawn_type);
        let at = match when_due(kind, alive, in_combat(map, g)) {
            Due::Despawn => {
                scripts::despawn(map, g);
                continue;
            }
            Due::Die => {
                if let Some(c) = map.creature_mut(g) {
                    c.fields.set_u32(UNIT_FIELD_HEALTH, 0);
                }
                crate::game::map::ai::die(map, g, g);
                None
            }
            // Fighting, it waits; the fight's end starts its time again.
            // A wild summon's death waits on the fight alone: it dies as
            // soon as that is over, its time not drawn out (Thistle Tea's
            // `TemporarySummon.next_at`, a tenth of a second on).
            Due::Wait if kind == TIMED_DEATH => Some(now + TIMED_DEATH_RECHECK),
            Due::Wait => Some(now + DESPAWN_RECHECK),
            Due::Ignore => None,
        };
        if let Some(s) = scripted_mut(map, g) {
            s.despawn_at = at;
        }
    }
    // Spawned events queued since the last update.
    let spawned = std::mem::take(&mut map.ai_spawned);
    for g in spawned {
        spawned_now(map, g);
    }
}

fn spawned_now(map: &mut Map, g: Guid) {
    let Some(c) = map.creature(g) else {
        return;
    };
    let (alive, summoner) = (c.alive(), c.scripted.as_deref().and_then(|s| s.summoner));
    if alive {
        crate::game::map::instance_script::creature_event(
            map,
            g,
            crate::game::map::instance_script::Event::Spawned,
        );
        if has_events(map, g) {
            ensure_started(map, g);
            fire(map, g, |e| e.kind == EVENT_SPAWNED, None);
        }
        // A Lunar Festival firework guy sends its firework up.
        crate::game::fireworks::spawned(map, g);
    }
    // Its summoner hears of it once it is in the world, its own spawned
    // events first (Thistle Tea's `handle_continue({:spawned, _})`).
    if let Some(s) = summoner.filter(|s| guid::is_unit(*s)) {
        summoned_unit(map, s, g);
    }
}

/// A creature came into the map: its spawned events run at the next
/// update.
pub fn spawned(map: &mut Map, g: Guid) {
    map.ai_spawned.push(g);
}

/// It entered combat with `enemy`.
pub fn aggro(map: &mut Map, g: Guid, enemy: Guid) {
    crate::game::map::instance_script::creature_event(
        map,
        g,
        crate::game::map::instance_script::Event::Aggro,
    );
    own_aggro(map, g, enemy);
    // Its group and links join in after its own events (Thistle Tea
    // tells them by an effect queued as it enters combat).
    crate::game::map::creature_groups::aggro(map, g, enemy);
}

fn own_aggro(map: &mut Map, g: Guid, enemy: Guid) {
    let now = map.now;
    if !has_events(map, g) {
        return;
    }
    ensure_started(map, g);
    let Some(entry) = map.creature(g).map(ai_entry) else {
        return;
    };
    let events = events_of(map, entry)
        .map(<[AiEvent]>::to_vec)
        .unwrap_or_default();
    if let Some(s) = scripted_mut(map, g) {
        reset_for_combat(s, &events, now);
        s.in_combat = true;
    }
    fire(map, g, |e| e.kind == EVENT_AGGRO, Some(enemy));
}

/// It died at `killer`'s hand.
pub fn death(map: &mut Map, g: Guid, killer: Guid) {
    if has_events(map, g) {
        fire(map, g, |e| e.kind == EVENT_DEATH, Some(killer));
    }
}

/// Its summoner and its group hear of its death, once it is dead (as
/// vmangos's `SummonedCreatureJustDied` comes): a summoner that asks
/// whether any of them is left alive doesn't count it.
pub fn death_heard(map: &mut Map, g: Guid) {
    // Its summoner hears of it.
    let summoner = scripted(map, g).and_then(|s| s.summoner);
    let entry = map.creature(g).map(|c| c.entry).unwrap_or(0);
    if let Some(s) = summoner
        && guid::is_unit(s)
    {
        fire(
            map,
            s,
            |e| e.kind == EVENT_SUMMONED_JUST_DIED && e.params[0] as u32 == entry,
            Some(g),
        );
    }
    // Its group hears of it.
    let members = crate::game::map::creature_groups::members_of(map, g);
    let leader = crate::game::map::creature_groups::is_leader(map, g);
    // The dead hear nothing (Thistle Tea's `{:member_died, ..}`).
    let alive = |map: &Map, m: Guid| map.creature(m).is_some_and(|c| c.alive());
    for m in members.into_iter().filter(|m| *m != g) {
        if !alive(map, m) || !crate::game::map::creature_groups::hears_death(map, m, g) {
            continue;
        }
        fire(
            map,
            m,
            |e| {
                e.kind == EVENT_GROUP_MEMBER_DIED
                    && (e.params[0] == 0 || e.params[0] as u32 == entry)
                    && (e.params[1] != 0) == leader
            },
            Some(g),
        );
    }
}

/// It killed `victim`.
pub fn kill(map: &mut Map, g: Guid, victim: Guid) {
    if has_events(map, g) {
        fire(map, g, |e| e.kind == EVENT_KILL, Some(victim));
    }
}

/// It gave up the fight.
pub fn evade(map: &mut Map, g: Guid) {
    crate::game::map::instance_script::creature_event(
        map,
        g,
        crate::game::map::instance_script::Event::Evade,
    );
    crate::game::map::creature_groups::evaded(map, g);
    let now = map.now;
    if !has_events(map, g) {
        return;
    }
    // Out of the fight, then evading, as Thistle Tea tells them.
    let was = scripted(map, g).is_some_and(|s| s.in_combat);
    if let Some(s) = scripted_mut(map, g) {
        s.in_combat = false;
    }
    if was {
        fire(map, g, |e| e.kind == EVENT_LEAVE_COMBAT, None);
    }
    fire(map, g, |e| e.kind == EVENT_EVADE, None);
    let Some(entry) = map.creature(g).map(ai_entry) else {
        return;
    };
    let events = events_of(map, entry)
        .map(<[AiEvent]>::to_vec)
        .unwrap_or_default();
    if let Some(s) = scripted_mut(map, g) {
        // The phase stays: vmangos keeps it through an evade, and scripts
        // that want it reset do so on their evade event.
        s.in_combat = false;
        if s.timers.len() == events.len() {
            reset_ooc(s, &events, now);
        }
    }
}

/// Its fight stopped (an evade, its death, a script's combat stop), as
/// Thistle Tea's `Engagement.leave`: a faction a script set for the fight
/// gives way, and a summon that waits out its fights (not dead) starts its
/// time again.
pub fn combat_stopped(map: &mut Map, g: Guid, died: bool) {
    scripts::restore_faction(map, g, FACTION_RESTORE_COMBAT_STOP);
    let now = map.now;
    if !died
        && let Some(s) = map
            .creature_mut(g)
            .filter(|c| c.spawn_guid.is_none())
            .and_then(|c| c.scripted.as_deref_mut())
        && WAITS_OUT_COMBAT.contains(&s.despawn_type)
        && let Some(after) = s.despawn_after
    {
        s.despawn_at = Some(now + after);
    }
}

/// It walked home from an evade.
pub fn reached_home(map: &mut Map, g: Guid) {
    if has_events(map, g) {
        fire(map, g, |e| e.kind == EVENT_REACHED_HOME, None);
    }
}

/// A spell of `caster`'s hit it.
pub fn spell_hit(map: &mut Map, g: Guid, caster: Guid, spell: u32, school_mask: u32) {
    if guid::is_unit(g) && has_events(map, g) {
        fire(
            map,
            g,
            |e| {
                e.kind == EVENT_HIT_BY_SPELL
                    && (e.params[0] == 0 || e.params[0] as u32 == spell)
                    && (e.params[1] <= 0 || e.params[1] as u32 & school_mask != 0)
            },
            Some(caster),
        );
    }
    if guid::is_unit(caster) && has_events(map, caster) {
        fire(
            map,
            caster,
            |e| {
                e.kind == EVENT_SPELL_HIT_TARGET
                    && (e.params[0] == 0 || e.params[0] as u32 == spell)
                    && (e.params[1] <= 0 || e.params[1] as u32 & school_mask != 0)
            },
            Some(g),
        );
    }
}

/// A player's text emote at it.
pub fn receive_emote(map: &mut Map, g: Guid, player: Guid, emote: u32) {
    if has_events(map, g) {
        fire(
            map,
            g,
            |e| e.kind == EVENT_RECEIVE_EMOTE && e.params[0] as u32 == emote,
            Some(player),
        );
    }
}

/// A script's event (`SCRIPT_COMMAND_SEND_SCRIPT_EVENT`).
pub fn script_event(map: &mut Map, g: Guid, event: u32, data: u32, invoker: Option<Guid>) {
    if has_events(map, g) {
        fire(
            map,
            g,
            |e| {
                e.kind == EVENT_SCRIPT_EVENT
                    && e.params[0] as u32 == event
                    && e.params[1] as u32 == data
            },
            invoker,
        );
    }
}

/// It summoned `summoned`.
pub fn summoned_unit(map: &mut Map, g: Guid, summoned: Guid) {
    let entry = map.creature(summoned).map(|c| c.entry).unwrap_or(0);
    if has_events(map, g) {
        fire(
            map,
            g,
            |e| e.kind == EVENT_SUMMONED_UNIT && e.params[0] as u32 == entry,
            Some(summoned),
        );
    }
}

/// A summoned creature of its went away.
pub fn summoned_despawned(map: &mut Map, g: Guid, summoned: Guid, entry: u32) {
    if has_events(map, g) {
        fire(
            map,
            g,
            |e| e.kind == EVENT_SUMMONED_JUST_DESPAWN && e.params[0] as u32 == entry,
            Some(summoned),
        );
    }
}

/// It reached a point of its movement: 2 for a waypoint. A dead one hears
/// nothing of it (Thistle Tea's `on_movement_inform`).
pub fn movement_inform(map: &mut Map, g: Guid, kind: u32, point: u32) {
    if has_events(map, g) && map.creature(g).is_some_and(|c| c.alive()) {
        fire(
            map,
            g,
            |e| {
                e.kind == EVENT_MOVEMENT_INFORM
                    && e.params[0] as u32 == kind
                    && e.params[1] as u32 == point
            },
            None,
        );
    }
}

/// How near its spot a follower counts as there, how far its spot moves
/// before it paths again, how far ahead it looks along its leader's way,
/// and the slack it keeps by a leader standing still (Thistle Tea's
/// `BT.Follow`).
const FOLLOW_START: f32 = 0.25;
const FOLLOW_REPATH: f32 = 0.5;
const FOLLOW_AHEAD: f32 = 0.5;
const FOLLOW_SLACK: f32 = 1.4;
const DEFAULT_RADIUS: f32 = 0.389;

/// Where a leader will be half a second on, which way it faces, and
/// whether it is on the move: a player by the way its client says it
/// goes, a creature where it is along its walk.
fn leader_ahead(map: &Map, leader: Guid) -> Option<(Vector3d, f32, bool)> {
    let now = map.now;
    if let Some(p) = map.players.get(&leader) {
        let info = &p.movement;
        let f = &info.flags;
        let axis = |a: bool, b: bool| f32::from(u8::from(a)) - f32::from(u8::from(b));
        let forward = axis(f.get_forward(), f.get_backward());
        let strafe = axis(f.get_strafe_left(), f.get_strafe_right());
        let airborne = f.get_jumping().is_some() || f.get_fallingfar();
        let o = info.orientation;
        let (dx, dy) = (
            forward * o.cos() - strafe * o.sin(),
            forward * o.sin() + strafe * o.cos(),
        );
        let len = (dx * dx + dy * dy).sqrt();
        let swimming = f.get_swimming().is_some();
        let speed = match (f.get_walk_mode(), forward < 0.0, swimming) {
            (true, _, _) => p.speeds.walk,
            (false, true, true) => p.speeds.swim_back,
            (false, true, false) => p.speeds.run_back,
            (false, false, true) => p.speeds.swim,
            (false, false, false) => p.speeds.run,
        };
        let at = info.position;
        if len == 0.0 || airborne || speed <= 0.0 {
            return Some((at, o, false));
        }
        let step = speed * FOLLOW_AHEAD / len;
        let ahead = Vector3d {
            x: at.x + dx * step,
            y: at.y + dy * step,
            z: at.z,
        };
        return Some((ahead, o, true));
    }
    let c = map.creature(leader)?;
    match &c.motion {
        crate::game::map::creature::Motion::Moving(w) if !w.done(now) => {
            let (at, o) = w.position_at(now);
            Some((at, o, true))
        }
        _ => Some((c.position, c.orientation, false)),
    }
}

/// A follower keeps its place by the one it follows, as Thistle Tea's
/// `BT.Follow` has it: at its distance and angle off the leader's facing
/// where the leader will be half a second on; running, faster the
/// further behind it is, and facing as the leader does when there. It
/// paths again only once that spot has moved, and settles by a leader who
/// stands. A leader out of sight is waited for.
pub fn follow_step(map: &mut Map, g: Guid) {
    let Some((leader, distance, angle)) = scripted(map, g).and_then(|s| s.follow) else {
        return;
    };
    let Some((ahead, facing, leader_moving)) = leader_ahead(map, leader) else {
        return;
    };
    crate::game::map::ai::advance_motion(map, g);
    let radius = |x: Guid| {
        map.unit_fields(x)
            .map(|f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
            .filter(|r| *r > 0.0)
            .unwrap_or(DEFAULT_RADIUS)
    };
    let slack = FOLLOW_SLACK * distance + radius(g) + radius(leader);
    let Some(c) = map.creature(g) else {
        return;
    };
    let a = facing + angle;
    let spot = Vector3d {
        x: ahead.x + distance * a.cos(),
        y: ahead.y + distance * a.sin(),
        z: ahead.z,
    };
    let heading = match &c.motion {
        crate::game::map::creature::Motion::Moving(w) => Some(w.destination()),
        _ => None,
    };
    let settled = !leader_moving && heading.is_none() && dist(c.position, ahead) <= slack;
    if settled
        || dist(c.position, spot) <= FOLLOW_START
        || heading.is_some_and(|h| dist(h, spot) <= FOLLOW_REPATH)
    {
        return;
    }
    // Further behind than it runs in a second, it runs faster, up to 2.1
    // times.
    let behind = dist(c.position, spot);
    let run = c.speeds.run;
    let factor = if behind > run {
        (1.0 + 0.04 * (behind - run)).min(2.1)
    } else {
        1.0
    };
    let from = c.position;
    let path = match map.nav.as_mut() {
        Some(nav) => nav.path(from, spot).unwrap_or_else(|| vec![from, spot]),
        None => vec![from, spot],
    };
    let now = map.now;
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let fly = crate::game::map::creature_motion::habitat(map, g).fly;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let mut walk = crate::game::map::creature::Walk::new(path, run * factor, now, spline_id, true);
    walk.fly = fly;
    walk.face = Some(facing);
    let frame = walk.frame(g, from);
    c.motion = crate::game::map::creature::Motion::Moving(walk);
    if let Some(s) = c.scripted.as_deref_mut() {
        s.scripted_move = None;
    }
    if let Some(frame) = frame {
        map.send_to_observers(g, &frame, false);
    }
}

/// The spell a script made its swings.
pub fn melee_spell(map: &Map, g: Guid) -> Option<u32> {
    scripted(map, g).and_then(|s| s.melee_spell)
}

/// Down in a linked feign death.
pub fn feigning(map: &Map, g: Guid) -> bool {
    scripted(map, g).is_some_and(|s| s.feign.is_some_and(|f| f.down_at.is_some()))
}

/// A killing blow on a creature with a linked feign death: it goes down
/// instead, at one health, and stops fighting.
pub fn feign_death(map: &mut Map, g: Guid) -> bool {
    let now = map.now;
    let Some(f) = scripted(map, g).and_then(|s| s.feign) else {
        return false;
    };
    if f.down_at.is_some() {
        return true;
    }
    crate::game::map::spells::interrupt(map, g);
    crate::game::map::combat::stop_attack(map, g);
    let held: Vec<u32> = map
        .creature(g)
        .map(|c| c.auras.holders.iter().map(|a| a.spell).collect())
        .unwrap_or_default();
    for spell in held {
        crate::game::map::auras::remove_spell(map, g, spell, None);
    }
    let at = map.position_of(g);
    map.note_fighting(g);
    if let Some(c) = map.creature_mut(g) {
        c.motion = crate::game::map::creature::Motion::Idle { until: None };
        c.fields.set_u32(UNIT_FIELD_HEALTH, 1);
        c.fields.set_byte(UNIT_FIELD_BYTES_1, 0, 7);
        let dynamic = c.fields.get_u32(UNIT_DYNAMIC_FLAGS);
        c.fields.set_u32(UNIT_DYNAMIC_FLAGS, dynamic | 0x20);
        if let Some(s) = c.scripted.as_deref_mut()
            && let Some(f) = s.feign.as_mut()
        {
            f.down_at = Some(now);
        }
    }
    if let Some(at) = at {
        let spline_id = map.next_spline_id;
        map.next_spline_id = map.next_spline_id.wrapping_add(1);
        let frame = crate::game::map::ai::stop_packet(g, at, spline_id);
        map.send_to_observers(g, &frame, false);
    }
    scripts::start(map, ScriptTable::Generic, f.fall_script, g, g);
    true
}

/// A creature down in a linked feign death: it gets up if a kin of its
/// fights on near it when its time is up, and dies if none does.
pub fn feign_tick(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(f) = scripted(map, g).and_then(|s| s.feign) else {
        return;
    };
    let Some(down) = f.down_at else {
        return;
    };
    let foes: Vec<Guid> = map
        .creature(g)
        .map(|c| c.threat.iter().map(|(t, _)| *t).collect())
        .unwrap_or_default();
    if !foes
        .iter()
        .any(|t| map.unit_view(*t).is_some_and(|u| u.alive))
    {
        feign_reset(map, g, f, true);
        return;
    }
    if now < down + f.delay {
        return;
    }
    let me = map.position_of(g).unwrap_or_default();
    let peer = map.creatures.iter().any(|(_, c)| {
        c.guid != g
            && c.entry == f.entry
            && c.alive()
            && c.combat.attacking
            && c.fields.get_u32(UNIT_FIELD_HEALTH) > 1
            && dist(c.position, me) <= f.radius
    });
    if peer {
        feign_reset(map, g, f, true);
        scripts::start(map, ScriptTable::Generic, f.revive_script, g, g);
        return;
    }
    feign_reset(map, g, f, false);
    if let Some(s) = scripted_mut(map, g) {
        s.feign = None;
    }
    let killer = foes.first().copied().unwrap_or(g);
    if let Some(c) = map.creature_mut(g) {
        c.fields.set_u32(UNIT_FIELD_HEALTH, 0);
    }
    crate::game::map::combat::kill(map, killer, g);
}

fn feign_reset(map: &mut Map, g: Guid, f: Feign, heal: bool) {
    if f.pacify != 0 {
        crate::game::map::auras::remove_spell(map, g, f.pacify, None);
    }
    let enemy = map
        .creature(g)
        .and_then(|c| c.threat.first().map(|(t, _)| *t));
    if let Some(c) = map.creature_mut(g) {
        c.fields.set_byte(UNIT_FIELD_BYTES_1, 0, 0);
        let dynamic = c.fields.get_u32(UNIT_DYNAMIC_FLAGS);
        c.fields.set_u32(UNIT_DYNAMIC_FLAGS, dynamic & !0x20);
        if heal {
            let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
            c.fields.set_u32(UNIT_FIELD_HEALTH, max);
        }
        if let Some(s) = c.scripted.as_deref_mut()
            && let Some(f) = s.feign.as_mut()
        {
            f.down_at = None;
        }
    }
    if heal && let Some(e) = enemy.filter(|e| map.unit_view(*e).is_some_and(|u| u.alive)) {
        crate::game::map::ai::attacked(map, g, e);
    }
}

/// A creature holding its path: let go when its summons since are gone,
/// when a script lets it, or when its time runs out.
pub fn hold_tick(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(h) = scripted(map, g).and_then(|s| s.hold.clone()) else {
        return;
    };
    let script = if now >= h.until {
        if !h.signaled && h.expired_script != 0 {
            h.expired_script
        } else {
            h.release_script
        }
    } else if h.signal || now < h.ready_at {
        return;
    } else {
        let live: Vec<Guid> = scripted(map, g)
            .map(|s| {
                s.summoned
                    .iter()
                    .copied()
                    .filter(|x| map.creature(*x).is_some_and(|c| c.alive()))
                    .collect()
            })
            .unwrap_or_default();
        if !live.iter().all(|x| h.earlier.contains(x)) {
            return;
        }
        h.release_script
    };
    if let Some(c) = map.creature_mut(g) {
        if let Some(s) = c.scripted.as_deref_mut() {
            s.hold = None;
        }
        if !matches!(c.motion, crate::game::map::creature::Motion::Moving(_)) {
            c.motion = crate::game::map::creature::Motion::Idle { until: Some(now) };
        }
    }
    scripts::start(map, ScriptTable::Generic, script, g, Guid::zero());
}

/// The phase scripts set.
pub fn set_phase(map: &mut Map, g: Guid, phase: u32) {
    if let Some(s) = scripted_mut(map, g) {
        s.phase = phase.min(31);
    }
}

pub fn phase(map: &Map, g: Guid) -> u32 {
    scripted(map, g).map_or(0, |s| s.phase)
}

/// Whether the creature chases: not if a script (or a main-ranged spell)
/// stopped it, nor, sessile, unless a script lets it (mangos's
/// `SetAIImmobilizedState`, Thistle Tea's `combat_movement?`).
pub fn combat_movement(map: &Map, g: Guid) -> bool {
    let s = scripted(map, g);
    !s.is_some_and(|s| s.no_combat_movement)
        && (s.is_some_and(|s| s.combat_movement_on)
            || !crate::game::map::creature_motion::sessile(map, g))
        && !crate::game::map::spells::lists::holding(map, g)
}

/// `static_flags1`: it doesn't swing (mangos's `NO_MELEE_FLEE`, the
/// combat dummies').
const STATIC_NO_MELEE: u32 = 0x0010_0000;

/// A creature whose template bars melee doesn't swing unless a script
/// lets it (Thistle Tea's `melee_enabled?`).
pub fn melee(map: &Map, g: Guid) -> bool {
    let s = scripted(map, g);
    let barred = map
        .creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .is_some_and(|t| t.static_flags1 & STATIC_NO_MELEE != 0);
    !s.is_some_and(|s| s.no_melee)
        && (s.is_some_and(|s| s.melee_on) || !barred)
        && !crate::game::map::spells::lists::holding(map, g)
}

/// `static_flags1`: it can't be killed (a floor of 1 health).
const STATIC_UNKILLABLE: u32 = 0x8;

/// Damage to an invincible creature leaves it its floor of health: the
/// one a script set (`SCRIPT_COMMAND_INVINCIBILITY`), else 1 for one its
/// template makes unkillable, as Thistle Tea's `invincibility_threshold`.
pub fn invincible_floor(map: &Map, g: Guid) -> u32 {
    if let Some(hp) = scripted(map, g).and_then(|s| s.invincible_hp) {
        return hp;
    }
    let unkillable = map
        .creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .is_some_and(|t| t.static_flags1 & STATIC_UNKILLABLE != 0);
    u32::from(unkillable)
}
