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
    pub no_melee: bool,
    pub no_combat_movement: bool,
    /// Health it can't be brought below.
    pub invincible_hp: u32,
    pub summoner: Option<Guid>,
    /// When a summoned creature goes, and how (vmangos's despawn types).
    pub despawn_at: Option<Instant>,
    pub despawn_type: i32,
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
}

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
}

/// The motion type of a script's point movement.
pub const POINT_MOTION: u32 = 9;

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
        .is_some_and(|c| events_of(map, c.entry).is_some())
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
    let Some(entry) = map.creature(g).map(|c| c.entry) else {
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
    for (i, e) in events.iter().enumerate() {
        if e.kind == EVENT_TIMER_OOC {
            s.timers[i] = Some(now + Duration::from_millis(roll(e.params[0], e.params[1])));
            s.disabled[i] = false;
        }
    }
}

fn reset_for_combat(s: &mut Scripted, events: &[AiEvent], now: Instant) {
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
    let Some(entry) = map
        .creature(g)
        .filter(|c| c.owner.is_none())
        .map(|c| c.entry)
    else {
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
    let actions: Vec<u32> = e.actions.iter().copied().filter(|a| *a != 0).collect();
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
    if let Some(o) = m.face {
        c.orientation = o;
        map.visibility_dirty.insert(g);
    }
    if let Some(point) = m.point {
        movement_inform(map, g, POINT_MOTION, point);
    }
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
    // The time since the last look, which a stun's pause adds back.
    let since = s
        .next_tick
        .and_then(|t| t.checked_sub(TICK))
        .map_or(TICK, |last| now.saturating_duration_since(last));
    s.next_tick = Some(now + TICK);
    // A stunned creature's paused timers wait out the stun.
    let paused: Vec<usize> = map
        .creature(g)
        .and_then(|c| events_of(map, c.entry))
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

/// A creature's despawn timer, for those summoned with one.
pub fn update_despawns(map: &mut Map) {
    let now = map.now;
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
        // Types: 1 timed or dead, 2 timed out of combat, 3 timed after
        // death, 4 dead, ... A creature fighting with type 2 waits.
        if kind == 2 && in_combat(map, g) {
            continue;
        }
        if kind == 3 && alive {
            if let Some(s) = scripted_mut(map, g) {
                s.despawn_at = None;
            }
            continue;
        }
        scripts::despawn(map, g);
    }
    // Spawned events queued since the last update.
    let spawned = std::mem::take(&mut map.ai_spawned);
    for g in spawned {
        spawned_now(map, g);
    }
}

fn spawned_now(map: &mut Map, g: Guid) {
    if map.creature(g).is_none_or(|c| !c.alive()) {
        return;
    }
    crate::game::map::instance_script::creature_event(
        map,
        g,
        crate::game::map::instance_script::Event::Spawned,
    );
    if has_events(map, g) {
        ensure_started(map, g);
        fire(map, g, |e| e.kind == EVENT_SPAWNED, None);
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
    crate::game::map::creature_groups::aggro(map, g, enemy);
    let now = map.now;
    if !has_events(map, g) {
        return;
    }
    ensure_started(map, g);
    let Some(entry) = map.creature(g).map(|c| c.entry) else {
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
    for m in members.into_iter().filter(|m| *m != g) {
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
    let Some(entry) = map.creature(g).map(|c| c.entry) else {
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

/// It reached a point of its movement: 2 for a waypoint.
pub fn movement_inform(map: &mut Map, g: Guid, kind: u32, point: u32) {
    if has_events(map, g) {
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

/// A follower keeps its place by the one it follows.
pub fn follow_step(map: &mut Map, g: Guid) {
    let Some((leader, distance, angle)) = scripted(map, g).and_then(|s| s.follow) else {
        return;
    };
    let Some(l) = map.unit_view(leader).filter(|l| l.alive) else {
        if let Some(s) = scripted_mut(map, g) {
            s.follow = None;
        }
        return;
    };
    let Some(c) = map.creature(g) else {
        return;
    };
    let a = l.orientation + angle;
    let spot = wow_world_messages::vanilla::Vector3d {
        x: l.position.x + distance * a.cos(),
        y: l.position.y + distance * a.sin(),
        z: l.position.z,
    };
    let heading = match &c.motion {
        crate::game::map::creature::Motion::Moving(w) => Some(w.destination()),
        _ => None,
    };
    if dist(c.position, spot) < 2.0 || heading.is_some_and(|h| dist(h, spot) < 2.0) {
        return;
    }
    let run = dist(c.position, spot) > 10.0 || scripted(map, g).is_some_and(|s| s.run);
    scripts::move_to(map, g, spot, run);
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

/// Whether a script (or a main-ranged spell) stopped the creature chasing
/// or swinging.
pub fn combat_movement(map: &Map, g: Guid) -> bool {
    !scripted(map, g).is_some_and(|s| s.no_combat_movement)
        && !crate::game::map::spells::lists::holding(map, g)
}

pub fn melee(map: &Map, g: Guid) -> bool {
    !scripted(map, g).is_some_and(|s| s.no_melee)
        && !crate::game::map::spells::lists::holding(map, g)
}

/// Damage to a scripted-invincible creature leaves it its floor of
/// health.
pub fn invincible_floor(map: &Map, g: Guid) -> u32 {
    scripted(map, g).map_or(0, |s| s.invincible_hp)
}
