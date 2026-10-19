//! Creature spell lists (`creature_spells`), as Thistle Tea runs them with
//! vmangos's meanings. Each entry has a timer of its own, rolled from its
//! initial delay at a fight's first try; the list is tried every 1.2
//! seconds while the creature isn't casting, in order, and the first entry
//! that is ready and casts wins. An entry whose cast wouldn't go (range,
//! sight, power, silence) is passed over with its timer as it was; its
//! chance is rolled once it would go, and a failed roll starts its repeat
//! delay. A main-ranged spell (0x08) that goes holds its caster where it
//! stands, neither chasing nor swinging, until one fails. Leaving the fight
//! puts it all back, and a list a script set gives way to the template's.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::SMSG_ATTACKSTOP;

use super::Targets;
use crate::game::data::scripts::ScriptTable;
use crate::game::data::spells::{CreatureSpell, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::{self, dist};

/// How often a creature tries its list.
pub const LIST_TICK: Duration = Duration::from_millis(1200);

// vmangos's cast flags, for lists and scripts alike.
pub const CF_INTERRUPT_PREVIOUS: u32 = 0x01;
pub const CF_TRIGGERED: u32 = 0x02;
pub const CF_FORCE_CAST: u32 = 0x04;
pub const CF_MAIN_RANGED: u32 = 0x08;
pub const CF_TARGET_UNREACHABLE: u32 = 0x10;
pub const CF_AURA_NOT_PRESENT: u32 = 0x20;
pub const CF_ONLY_IN_MELEE: u32 = 0x40;
pub const CF_NOT_IN_MELEE: u32 = 0x80;
pub const CF_TARGET_CASTING: u32 = 0x100;

/// Where the most injured friend is looked for without a radius of the
/// entry's or a range of the spell's, and how hurt it must be without a
/// threshold of the entry's.
const INJURED_RADIUS: f32 = 30.0;
const INJURED_THRESHOLD: u32 = 50;

/// A creature's place in its list, for the fight it is in.
#[derive(Debug, Clone, Default)]
pub struct ListState {
    /// The list the timers are for.
    pub list: u32,
    /// When each entry is next ready.
    pub ready_at: Vec<Instant>,
    /// When the list is next tried.
    pub next_at: Option<Instant>,
    /// Held at range by a main-ranged spell: no chase, no melee.
    pub holding: bool,
}

/// The list a creature casts from: one a script set (0 for none) over its
/// own.
pub fn list_of(map: &Map, g: Guid) -> u32 {
    map.creature(g)
        .and_then(|c| c.scripted.as_deref())
        .and_then(|s| s.spell_list)
        .unwrap_or_else(|| base_list(map, g))
}

/// A creature's own list: its template's, or one it kept from an entry it
/// was before (`update_entry` to an entry with none).
pub fn base_list(map: &Map, g: Guid) -> u32 {
    let Some(c) = map.creature(g) else {
        return 0;
    };
    c.scripted
        .as_deref()
        .and_then(|s| s.base_spell_list)
        .or_else(|| {
            map.realm
                .data
                .creature_template(c.entry)
                .map(|t| t.spell_list_id)
        })
        .unwrap_or(0)
}

/// Whether a main-ranged spell holds `g` at range.
pub fn holding(map: &Map, g: Guid) -> bool {
    map.creature(g)
        .and_then(|c| c.caster.spell_list.as_deref())
        .is_some_and(|s| s.holding)
}

/// It left its fight: its timers and its hold go, and a list a script set
/// gives way to its template's.
pub fn left_combat(map: &mut Map, g: Guid) {
    if let Some(c) = map.creature_mut(g) {
        c.caster.spell_list = None;
        if let Some(s) = c.scripted.as_deref_mut() {
            s.spell_list = None;
        }
    }
}

/// A script set list `list` (0 for none): its timers start over from its
/// initial delays.
pub fn set_list(map: &mut Map, g: Guid, list: u32) {
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.spell_list = Some(list);
    }
    if let Some(s) = map
        .creature_mut(g)
        .and_then(|c| c.caster.spell_list.as_deref_mut())
    {
        s.ready_at.clear();
    }
}

/// A delay between `lo` and `hi` milliseconds.
pub(crate) fn roll((lo, hi): (u32, u32)) -> Duration {
    let ms = if hi > lo {
        rand::rng().random_range(lo..=hi)
    } else {
        lo
    };
    Duration::from_millis(u64::from(ms))
}

/// An entry's chance in a hundred: a probability of none, or over a
/// hundred, is a hundred (Thistle Tea's `CreatureSpell.build`).
pub(crate) fn chance_of(probability: u32) -> u32 {
    if probability == 0 || probability > 100 {
        100
    } else {
        probability
    }
}

/// A creature fighting `victim` tries its list.
pub fn tick(map: &mut Map, g: Guid, victim: Guid) {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(c) = map.creature(g) else {
        return;
    };
    if c.caster.casting.is_some() || crate::game::map::auras::held(map, g, false) {
        return;
    }
    let id = list_of(map, g);
    let Some(list) = data
        .spells
        .creature_spells
        .get(&id)
        .filter(|l| id != 0 && !l.is_empty())
    else {
        return;
    };
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let state = c.caster.spell_list.get_or_insert_with(Default::default);
    if state.list != id || state.ready_at.len() != list.len() {
        state.list = id;
        state.ready_at = list.iter().map(|e| now + roll(e.delay_initial)).collect();
        state.next_at = None;
    }
    if state.next_at.is_some_and(|t| t > now) {
        return;
    }
    state.next_at = Some(now + LIST_TICK);
    let ready: Vec<bool> = state.ready_at.iter().map(|t| *t <= now).collect();
    for (i, e) in list.iter().enumerate() {
        if ready[i] && attempt(map, g, victim, i, e) {
            break;
        }
    }
}

/// Entry `i` tried: whether it was cast.
fn attempt(map: &mut Map, g: Guid, victim: Guid, i: usize, e: &CreatureSpell) -> bool {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(e.spell) else {
        return false;
    };
    let Some(target) = target_of(map, g, victim, e, s) else {
        return false;
    };
    if !flags_allow(map, g, e.flags, e.spell, target) {
        return false;
    }
    let targets = Targets {
        unit: Some(target),
        dest: map.position_of(target),
        ..Default::default()
    };
    let m = super::mods::for_spell(map, g, s);
    if super::checks::check(map, g, s, &targets, None, false, &m).is_some() {
        resume(map, g, e);
        return false;
    }
    let chance = rand::rng().random_range(0..100) < chance_of(e.probability);
    if let Some(t) = map
        .creature_mut(g)
        .and_then(|c| c.caster.spell_list.as_deref_mut())
        .and_then(|s| s.ready_at.get_mut(i))
    {
        *t = now + roll(e.delay_repeat);
    }
    if !chance {
        resume(map, g, e);
        return false;
    }
    if e.flags & CF_MAIN_RANGED != 0 {
        hold(map, g, victim);
    }
    // Its script runs as the cast begins, as Thistle Tea's
    // `run_cast_script`: before an instant spell lands, as a spell with a
    // cast time starts.
    let instant = data.spells.cast_time(s) == 0 && !s.channeled();
    if instant && e.script != 0 {
        crate::game::map::scripts::start(map, ScriptTable::CreatureSpells, e.script, g, target);
    }
    // A cast that takes time is cast standing, faced at its target: a
    // walk under way stops where it is (Thistle Tea's `try_cast`
    // stopping its spline and facing its victim).
    if !instant && target != g {
        if map
            .creature(g)
            .is_some_and(|c| matches!(c.motion, crate::game::map::creature::Motion::Moving(_)))
        {
            crate::game::map::ai::halt(map, g);
        }
        crate::game::map::ai::face(map, g, target);
    }
    if super::cast(map, g, e.spell, targets, None) && !instant && e.script != 0 {
        crate::game::map::scripts::start(map, ScriptTable::CreatureSpells, e.script, g, target);
    }
    true
}

/// A main-ranged spell went: it stands, and stops swinging (its next
/// swing in reach starts its attack again, as Thistle Tea's
/// `maybe_hold_ranged` clears `attack_started`).
fn hold(map: &mut Map, g: Guid, victim: Guid) {
    if let Some(c) = map.creature_mut(g) {
        c.combat.announced = None;
        if let Some(s) = c.caster.spell_list.as_deref_mut() {
            s.holding = true;
        }
    }
    if victim.is_zero() {
        return;
    }
    if let Some(frame) = crate::game::conn::encode(&SMSG_ATTACKSTOP {
        player: g,
        enemy: victim,
        unknown1: 0,
    }) {
        map.send_to_observers(g, &frame, false);
    }
}

/// A main-ranged spell failed: it chases and swings again, its
/// `SMSG_ATTACKSTART` going out at its first swing in reach.
fn resume(map: &mut Map, g: Guid, e: &CreatureSpell) {
    if e.flags & CF_MAIN_RANGED == 0 {
        return;
    }
    if let Some(s) = map
        .creature_mut(g)
        .and_then(|c| c.caster.spell_list.as_deref_mut())
    {
        s.holding = false;
    }
}

/// Whom entry `e` is cast at: itself (0); its victim (1 to 7); the most
/// injured friend near, itself among them (17) or not (18); itself if it
/// lacks the buff (19). The rest (16, 20) are never cast, as Thistle Tea
/// has it.
fn target_of(
    map: &Map,
    g: Guid,
    victim: Guid,
    e: &CreatureSpell,
    s: &SpellTemplate,
) -> Option<Guid> {
    match e.target {
        0 => Some(g),
        1..=7 => (!victim.is_zero()).then_some(victim),
        17 | 18 => injured_friend(map, g, e, s, e.target == 18),
        19 => {
            let buff = if e.target_param2 > 0 {
                e.target_param2
            } else {
                e.spell
            };
            (!crate::game::map::auras::auras(map, g).is_some_and(|a| a.has_spell(buff)))
                .then_some(g)
        }
        _ => None,
    }
}

/// How much of its health a unit lacks, in percent.
fn missing_pct(map: &Map, g: Guid) -> Option<f32> {
    let f = map.unit_fields(g)?;
    let max = f.get_u32(UNIT_FIELD_MAXHEALTH).max(1) as f32;
    Some(100.0 - f.get_u32(UNIT_FIELD_HEALTH) as f32 * 100.0 / max)
}

/// Whether `a` and `b` are friends: one side, or friendly factions.
fn friendly(map: &Map, a: Guid, b: Guid) -> bool {
    use crate::game::map::pets::master;
    if master(map, a) == master(map, b) {
        return true;
    }
    let faction = |g| {
        map.unit_fields(g)
            .map(|f| f.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
    };
    let templates = &map.realm.data.faction_templates;
    match (
        faction(a).and_then(|f| templates.get(&f)),
        faction(b).and_then(|f| templates.get(&f)),
    ) {
        (Some(fa), Some(fb)) => fa.friendly_to(fb),
        _ => false,
    }
}

/// The friend near `g` most hurt past the entry's threshold: within its
/// first parameter's yards (or the spell's range, or 30), lacking more
/// than its second parameter's percent (or half) of its health, and, but
/// for `g` itself, fighting and selectable.
fn injured_friend(
    map: &Map,
    g: Guid,
    e: &CreatureSpell,
    s: &SpellTemplate,
    except_self: bool,
) -> Option<Guid> {
    let at = map.position_of(g)?;
    let radius = if e.target_param1 > 0 {
        e.target_param1 as f32
    } else {
        let (_, max) = map.realm.data.spells.range(s);
        if s.range_index != 0 && max > 0.0 {
            max
        } else {
            INJURED_RADIUS
        }
    };
    let threshold = if (1..=100).contains(&e.target_param2) {
        e.target_param2
    } else {
        INJURED_THRESHOLD
    } as f32;
    let mut best: Option<(Guid, f32)> = None;
    let mut consider = |u: Guid, missing: f32| {
        if missing > threshold && best.is_none_or(|(_, b)| missing > b) {
            best = Some((u, missing));
        }
    };
    if !except_self
        && map.unit_view(g).is_some_and(|u| u.alive)
        && let Some(missing) = missing_pct(map, g)
    {
        consider(g, missing);
    }
    for u in map.grid.around(at.x, at.y, radius) {
        if u == g || !(guid::is_unit(u) || guid::is_player(u)) {
            continue;
        }
        let Some(view) = map.unit_view(u) else {
            continue;
        };
        let flags = map
            .unit_fields(u)
            .map_or(0, |f| f.get_u32(UNIT_FIELD_FLAGS));
        if !view.alive
            || dist(view.position, at) > radius
            || flags & crate::game::map::player::UNIT_FLAG_IN_COMBAT == 0
            || flags & 0x0200_0000 != 0
            || !friendly(map, g, u)
        {
            continue;
        }
        if let Some(missing) = missing_pct(map, u) {
            consider(u, missing);
        }
    }
    best.map(|(u, _)| u)
}

/// Whether `caster`'s cast flags let `spell` go at `target`: none while it
/// runs to keep its distance (movement 19) but one forced (0x04), as
/// Thistle Tea's `movement_allows_cast?`; never one that wants its target
/// out of reach (0x10) or casting (0x100); not an aura already there
/// (0x20); only in melee (0x40), or only out of it (0x80).
pub fn flags_allow(map: &Map, caster: Guid, flags: u32, spell: u32, target: Guid) -> bool {
    if flags & CF_FORCE_CAST == 0 && distancing(map, caster) {
        return false;
    }
    if flags & (CF_TARGET_UNREACHABLE | CF_TARGET_CASTING) != 0 {
        return false;
    }
    if flags & CF_AURA_NOT_PRESENT != 0
        && crate::game::map::auras::auras(map, target).is_some_and(|a| a.has_spell(spell))
    {
        return false;
    }
    if flags & CF_ONLY_IN_MELEE != 0 {
        return in_melee(map, caster, target);
    }
    if flags & CF_NOT_IN_MELEE != 0 {
        return !in_melee(map, caster, target);
    }
    true
}

/// Whether a creature is running to keep its distance (movement 19).
pub fn distancing(map: &Map, g: Guid) -> bool {
    map.creature(g).is_some_and(|c| {
        let Some(m) = c.scripted.as_deref().and_then(|s| s.scripted_move) else {
            return false;
        };
        m.distancing.is_some()
            && matches!(&c.motion, crate::game::map::creature::Motion::Moving(w) if w.spline_id == m.spline_id)
    })
}

fn in_melee(map: &Map, a: Guid, b: Guid) -> bool {
    a != b
        && match (map.unit_view(a), map.unit_view(b)) {
            (Some(x), Some(y)) => combat::in_melee_range(&x, &y),
            _ => false,
        }
}
