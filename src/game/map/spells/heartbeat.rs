//! Crowd control breaking early, as Thistle Tea's `Aura.Heartbeat` (after
//! vmangos) has it, for a harmful aura of more than ten seconds whose spell
//! checks: on a player (or what a player controls) it ends at a time drawn
//! from vmangos's spread, about twelve seconds in (sooner with diminishing
//! returns); on a creature (a player's pet or charmed one too) it rolls the
//! spell's hit again every five seconds and ends on a miss (not for fear,
//! roots, silences or confusion). It breaks before its own ticks due then.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras;

/// `SPELL_ATTR_HEARTBEAT_RESIST_CHECK`.
const ATTR_HEARTBEAT_RESIST: u32 = 0x4000_0000;
const INTERVAL: Duration = Duration::from_secs(5);
/// Fear, roots, pacify and silence, and confusion: not rolled on creatures.
const CREATURE_EXCLUSIONS: [u32; 4] = [7, 26, 60, 5];

/// When a holder's crowd control may break.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heartbeat {
    /// A player's: it ends then.
    pub break_at: Option<Instant>,
    /// A creature's: the next roll, and the chance to stay (hundredths of a
    /// percent).
    pub next_check: Option<Instant>,
    pub hit_chance: i32,
    /// A player's drawn delay before diminishing returns, in milliseconds,
    /// and the rate they cut it by: a refresh keeps the delay.
    pub player_delay: Option<f64>,
    pub rate: f64,
}

/// Whether `s` breaks early.
pub fn checks(s: &SpellTemplate) -> bool {
    matches!(s.id, 13181 | 13327) || s.attr(0, ATTR_HEARTBEAT_RESIST)
}

fn player_controlled(map: &Map, g: Guid) -> bool {
    if guid::is_player(g) {
        return true;
    }
    crate::game::map::pets::owner_of(map, g).is_some_and(guid::is_player)
        || map
            .unit_fields(g)
            .is_some_and(|f| guid::is_player(f.get_guid(UNIT_FIELD_CHARMEDBY)))
}

/// The heartbeat a new holder of `s` from `caster` on `target` lasting
/// `duration` gets, if any.
pub fn prepare(
    map: &Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    positive: bool,
    duration: Option<u32>,
) -> Option<Heartbeat> {
    let ms = duration?;
    if positive || ms <= 10_000 || !checks(s) {
        return None;
    }
    let now = map.now;
    let mut beat = Heartbeat {
        break_at: None,
        next_check: None,
        hit_chance: 10_000,
        player_delay: None,
        rate: 1.0,
    };
    if player_controlled(map, target) {
        let sample: f64 = if super::outcome::rolling(map) {
            rand::rng().random_range(f64::EPSILON..1.0)
        } else {
            0.5
        };
        let delay = (12_000.0 + 3_000.0 / 99f64.ln() * (sample / (1.0 - sample)).ln()).max(0.0);
        let rate = rate(map, caster, s, ms);
        beat.break_at = Some(now + Duration::from_millis((delay * rate) as u64));
        beat.player_delay = Some(delay);
        beat.rate = rate;
    }
    // A creature rolls too, a player's pet or charmed one as well (Thistle
    // Tea's `creature_hit_chance` is any mob's).
    if !guid::is_player(target)
        && s.attr(0, ATTR_HEARTBEAT_RESIST)
        && !s
            .effects
            .iter()
            .any(|e| CREATURE_EXCLUSIONS.contains(&e.aura))
    {
        let mods = super::mods::for_spell(map, caster, s);
        beat.next_check = Some(now + INTERVAL);
        // A spell that always hits always holds.
        beat.hit_chance = if s.attr(3, crate::game::data::spells::ATTR_EX3_ALWAYS_HIT) {
            10_000
        } else {
            super::outcome::spell_hit_chance(map, caster, target, s, &mods)
        };
    }
    (beat.break_at.is_some() || beat.next_check.is_some()).then_some(beat)
}

/// The share of `s`'s full duration that `ms` is: diminished returns
/// shorten a player's break alike.
fn rate(map: &Map, caster: Guid, s: &SpellTemplate, ms: u32) -> f64 {
    let level = map.unit_view(caster).map_or(1, |u| u.level);
    let full = map
        .realm
        .data
        .spells
        .duration(s, level)
        .unwrap_or(ms)
        .max(1);
    (f64::from(ms) / f64::from(full)).min(1.0)
}

/// A holder of `s` from `caster` refreshed, to last `duration`: a player's
/// keeps its drawn delay, started again from now at the new rate; a
/// creature's keeps its next roll and chance. Refreshed too short to check
/// anew (diminished returns), it keeps the old (Thistle Tea's holder keeps
/// its heartbeat, rescheduled).
pub fn refreshed(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    old: Option<Heartbeat>,
    new: Option<Heartbeat>,
    duration: Option<u32>,
) -> Option<Heartbeat> {
    let now = map.now;
    let Some(mut new) = new else {
        let (mut old, ms) = (old?, duration?);
        if let Some(delay) = old.player_delay {
            old.rate = rate(map, caster, s, ms);
            old.break_at = Some(now + Duration::from_millis((delay * old.rate) as u64));
        }
        return Some(old);
    };
    let Some(old) = old else {
        return Some(new);
    };
    if let (Some(delay), Some(_)) = (old.player_delay, new.player_delay) {
        new.player_delay = Some(delay);
        new.break_at = Some(now + Duration::from_millis((delay * new.rate) as u64));
    }
    if old.next_check.is_some() && new.next_check.is_some() {
        new.next_check = old.next_check;
        new.hit_chance = old.hit_chance;
    }
    Some(new)
}

fn due(b: &Heartbeat, now: Instant) -> bool {
    b.break_at.is_some_and(|t| t <= now) || b.next_check.is_some_and(|t| t <= now)
}

/// Whether any of the crowd control in `a` breaks or rolls now.
pub fn any_due(a: &auras::Auras, now: Instant) -> bool {
    a.holders
        .iter()
        .any(|h| h.heartbeat.is_some_and(|b| due(&b, now)))
}

/// `g`'s crowd control that breaks now.
pub fn update(map: &mut Map, g: Guid) {
    let now = map.now;
    let due: Vec<(u32, Guid, Heartbeat)> = auras::auras(map, g)
        .map(|a| {
            a.holders
                .iter()
                .filter_map(|h| h.heartbeat.map(|b| (h.spell, h.caster, b)))
                .filter(|(_, _, b)| due(b, now))
                .collect()
        })
        .unwrap_or_default();
    for (spell, caster, b) in due {
        let breaks =
            b.break_at.is_some_and(|t| t <= now) || !super::outcome::hits(map, b.hit_chance);
        if breaks {
            auras::remove_spell(map, g, spell, Some(caster));
        } else if let Some(a) = auras::auras_mut(map, g)
            && let Some(h) = a
                .holders
                .iter_mut()
                .find(|h| h.spell == spell && h.caster == caster)
            && let Some(hb) = h.heartbeat.as_mut()
            && let Some(at) = hb.next_check
        {
            let behind = now.saturating_duration_since(at).as_millis() / INTERVAL.as_millis();
            hb.next_check = Some(at + INTERVAL * (behind as u32 + 1));
        }
    }
}
