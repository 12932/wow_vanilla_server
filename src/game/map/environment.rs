//! The environment's harm, as Thistle Tea's `Core.Environment` has it:
//! breath under water (a minute, then a fifth of health every two
//! seconds), fatigue in the deep sea, lava burning after a second in it,
//! and the spells liquids put on (Naxxramas's slime); and going into deep
//! enough water, or out of it, ends what water breaks. The liquid's kind
//! comes from a WMO where one holds it, else from the terrain tiles; the
//! surface breath is held under, from the navmesh.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_MOD_WATER_BREATHING, AURA_WATER_BREATHING};
use crate::game::map::death::Death;
use crate::game::map::terrain::{Liquid, SPELL_NAXX_SLIME};

pub const BREATH: Duration = Duration::from_secs(60);
pub const FATIGUE: Duration = Duration::from_secs(60);
/// How long lava is touched before it burns.
pub const LAVA_GRACE: Duration = Duration::from_secs(1);
const PULSE: Duration = Duration::from_secs(2);
/// How often a player's liquid is looked at.
const CHECK_EVERY: Duration = Duration::from_millis(500);
/// How deep the head is: two yards at scale 1.
const BODY_HEIGHT: f32 = 2.0;

/// A mirror timer: what is left, the whole, and whether it drains (-1) or
/// fills again (10).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mirror {
    pub remaining: Duration,
    pub duration: Duration,
    pub draining: bool,
    pub updated: Instant,
    pub next_damage: Option<Instant>,
}

impl Mirror {
    fn new(duration: Duration, now: Instant) -> Self {
        Self {
            remaining: duration,
            duration,
            draining: true,
            updated: now,
            next_damage: None,
        }
    }

    /// Bring the timer up to `now`.
    fn advance(&mut self, now: Instant) {
        let dt = now.saturating_duration_since(self.updated);
        self.remaining = if self.draining {
            self.remaining.saturating_sub(dt)
        } else {
            (self.remaining + dt * 10).min(self.duration)
        };
        self.updated = now;
    }

    fn scale(&self) -> u32 {
        if self.draining { u32::MAX } else { 10 }
    }
}

fn start_frame(timer: TimerType, m: &Mirror) -> Option<std::sync::Arc<[u8]>> {
    crate::game::conn::encode(&SMSG_START_MIRROR_TIMER {
        timer,
        time_remaining: m.remaining.as_millis() as u32,
        duration: m.duration.as_millis() as u32,
        scale: m.scale(),
        is_frozen: false,
        id: 0,
    })
}

/// Environmental harm: logged, and dealt.
pub fn damage(map: &mut Map, g: Guid, kind: EnvironmentalDamageType, amount: u32) {
    let resist_school = match kind {
        EnvironmentalDamageType::Lava | EnvironmentalDamageType::Fire => Some(2u16),
        EnvironmentalDamageType::Slime => Some(3),
        _ => None,
    };
    let resist = resist_school
        .and_then(|s| {
            map.unit_fields(g)
                .map(|f| f.get_u32(UNIT_FIELD_RESISTANCES + s))
        })
        .map(|r| (amount as f32 * (r as f32 / (r as f32 + 300.0)).min(0.75)) as u32)
        .unwrap_or(0);
    let (left, absorbed) = match resist_school {
        Some(s) => crate::game::map::auras::absorb(map, g, amount - resist, 1 << s),
        None => (amount, 0),
    };
    if let Some(frame) = crate::game::conn::encode(&SMSG_ENVIRONMENTAL_DAMAGE_LOG {
        guid: g,
        damage_type: kind,
        damage: left,
        absorb: absorbed,
        resist,
    }) {
        map.send_to_observers(g, &frame, true);
    }
    if left > 0 {
        crate::game::map::combat::deal_damage(map, g, g, left);
    }
}

/// The liquid at a point, as Thistle Tea's `terrain_liquid` has it: a
/// WMO's, else the terrain tile's.
pub fn liquid_at(map: &mut Map, at: Vector3d) -> Option<Liquid> {
    if let Some((entry, surface, floor)) = map.nav.as_mut().and_then(|n| n.wmo_liquid(at)) {
        return Liquid::from_wmo(entry, surface, floor);
    }
    map.terrain.as_mut()?.liquid(at)
}

/// The surface over a point: the navmesh's liquid, of any kind; else that
/// of the liquid there.
fn surface_at(map: &mut Map, at: Vector3d, liquid: Option<Liquid>) -> Option<f32> {
    map.nav
        .as_mut()
        .and_then(|n| n.liquid_surface(at))
        .or(liquid.map(|l| l.surface))
}

/// Every player's breath, fatigue, burning and liquid spell.
pub fn update(map: &mut Map) {
    let now = map.now;
    let due: Vec<Guid> = map
        .players
        .values()
        .filter(|p| {
            p.misc
                .env_checked_at
                .is_none_or(|t| now.duration_since(t) >= CHECK_EVERY)
        })
        .map(|p| p.guid)
        .collect();
    for g in due {
        let Some(p) = map.players.get_mut(&g) else {
            continue;
        };
        p.misc.env_checked_at = Some(now);
        let alive = p.death == Death::Alive;
        let ghost = matches!(p.death, Death::Ghost { .. });
        // A ghost still tires in the deep sea; a body lying there doesn't.
        let protected = p.misc.god
            || p.flight.is_some()
            || p.movement.flags.get_on_transport().is_some()
            || !(alive || ghost);
        let at = p.position();
        let scale = p.fields.get_f32(OBJECT_FIELD_SCALE_X).max(0.1);
        let breath_for = if p.auras.has(AURA_WATER_BREATHING) {
            Duration::ZERO
        } else {
            let factor = p.auras.product(AURA_MOD_WATER_BREATHING);
            Duration::try_from_secs_f32(BREATH.as_secs_f32() * factor).unwrap_or(BREATH)
        };
        let liquid = liquid_at(map, at);
        liquid_spell(map, g, liquid.filter(|_| alive).and_then(|l| l.spell(at.z)));
        if alive {
            water(map, g, liquid.is_some_and(|l| swimmable(&l, at.z, scale)));
        }
        if protected {
            stop(map, g);
            continue;
        }
        match liquid {
            Some(l) => fatigue(map, g, l.deep(), now),
            None => stop_fatigue(map, g),
        }
        if !alive {
            stop_breath(map, g);
            lava(map, g, None, now);
            continue;
        }
        let depth = surface_at(map, at, liquid).map_or(0.0, |surface| surface - at.z);
        let submerged = depth > BODY_HEIGHT * scale;
        breathe(
            map,
            g,
            submerged && breath_for > Duration::ZERO,
            breath_for,
            depth > 0.0,
            now,
        );
        lava(map, g, liquid.map(|l| l.magma() && l.touching(at.z)), now);
    }
}

/// Whether someone at `z` is in `l` deep enough to swim, as mangos-classic
/// has it (`ENVIRONMENT_FLAG_HIGH_LIQUID`): under its surface, where it is
/// over three quarters of their height deep.
pub fn swimmable(l: &Liquid, z: f32, scale: f32) -> bool {
    z < l.surface && l.surface - l.floor > 0.75 * BODY_HEIGHT * scale
}

/// Into the water or out of it by the terrain's liquid: what water breaks
/// ends, as when the client's swimming flag changes (Thistle Tea's
/// `interrupt_water_auras`), for a client that doesn't say it swims.
fn water(map: &mut Map, g: Guid, in_water: bool) {
    use crate::game::map::auras::{INTERRUPT_ABOVE_WATER, INTERRUPT_UNDER_WATER};
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.misc.in_water == in_water {
        return;
    }
    p.misc.in_water = in_water;
    let mask = if in_water {
        INTERRUPT_UNDER_WATER
    } else {
        INTERRUPT_ABOVE_WATER
    };
    crate::game::map::auras::interrupt(map, g, mask);
}

fn stop(map: &mut Map, g: Guid) {
    stop_breath(map, g);
    stop_fatigue(map, g);
    if let Some(p) = map.players.get_mut(&g) {
        p.misc.lava = None;
    }
}

fn stop_breath(map: &mut Map, g: Guid) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.misc.breath.take().is_some() {
        p.outbound.send(&SMSG_STOP_MIRROR_TIMER {
            timer: TimerType::Breath,
        });
    }
}

fn stop_fatigue(map: &mut Map, g: Guid) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.misc.fatigue.take().is_some() {
        p.outbound.send(&SMSG_STOP_MIRROR_TIMER {
            timer: TimerType::Fatigue,
        });
    }
}

/// A fifth of the most health, and up to the level more.
fn fifth(p: &crate::game::map::player::Player) -> u32 {
    let level = p.fields.get_u32(UNIT_FIELD_LEVEL).max(1);
    let bonus = rand::rng().random_range(0..level);
    (p.fields.get_u32(UNIT_FIELD_MAXHEALTH) / 5 + bonus).max(1)
}

/// The breath timer: draining with the head under, filling above it,
/// gone when full or out of the water.
pub fn breathe(
    map: &mut Map,
    g: Guid,
    under: bool,
    duration: Duration,
    in_water: bool,
    now: Instant,
) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let before = p.misc.breath;
    let timer = match before {
        None if under => Some(Mirror::new(duration, now)),
        None => None,
        Some(mut m) => {
            m.advance(now);
            m.draining = under;
            if !under {
                m.next_damage = None;
            }
            (under || (in_water && m.remaining < m.duration)).then_some(m)
        }
    };
    p.misc.breath = timer;
    match (before, timer) {
        (_, Some(m)) if before.is_none_or(|b| b.draining != m.draining) => {
            if let Some(frame) = start_frame(TimerType::Breath, &m) {
                p.outbound.send_frame(frame);
            }
        }
        (Some(_), None) => {
            p.outbound.send(&SMSG_STOP_MIRROR_TIMER {
                timer: TimerType::Breath,
            });
        }
        _ => {}
    }
    let drowning = timer.is_some_and(|m| m.draining && m.remaining.is_zero());
    if drowning && timer.is_some_and(|m| m.next_damage.is_none_or(|t| t <= now)) {
        let amount = fifth(p);
        if let Some(m) = p.misc.breath.as_mut() {
            m.next_damage = Some(now + PULSE);
        }
        damage(map, g, EnvironmentalDamageType::Drowning, amount);
    }
}

/// The fatigue timer, in the deep sea; filling in other liquid. A ghost
/// worn out goes to its graveyard.
pub fn fatigue(map: &mut Map, g: Guid, deep: bool, now: Instant) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let before = p.misc.fatigue;
    let timer = match before {
        None if deep => Some(Mirror::new(FATIGUE, now)),
        None => None,
        Some(mut m) => {
            m.advance(now);
            m.draining = deep;
            (deep || m.remaining < m.duration).then_some(m)
        }
    };
    p.misc.fatigue = timer;
    match (before, timer) {
        (_, Some(m)) if before.is_none_or(|b| b.draining != m.draining) => {
            if let Some(frame) = start_frame(TimerType::Fatigue, &m) {
                p.outbound.send_frame(frame);
            }
        }
        (Some(_), None) => {
            p.outbound.send(&SMSG_STOP_MIRROR_TIMER {
                timer: TimerType::Fatigue,
            });
        }
        _ => {}
    }
    let exhausted = timer.is_some_and(|m| m.draining && m.remaining.is_zero());
    if exhausted && timer.is_some_and(|m| m.next_damage.is_none_or(|t| t <= now)) {
        if matches!(p.death, Death::Ghost { .. }) {
            let (zone, team, pos) = (p.zone, p.team, p.position());
            stop_fatigue(map, g);
            let (to, at, facing) = crate::game::map::death::nearest_graveyard(map, zone, team, pos);
            map.teleport(g, to, at, facing);
            return;
        }
        let amount = fifth(p);
        if let Some(m) = p.misc.fatigue.as_mut() {
            m.next_damage = Some(now + PULSE);
        }
        damage(map, g, EnvironmentalDamageType::Exhausted, amount);
    }
}

/// Lava: a second's grace touching it, then 605 to 610 fire every two
/// seconds; the grace comes back ten times as fast out of it. `touching`
/// is `None` out of any liquid.
pub fn lava(map: &mut Map, g: Guid, touching: Option<bool>, now: Instant) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(touching) = touching else {
        p.misc.lava = None;
        return;
    };
    let timer = match p.misc.lava {
        None if touching => Some(Mirror::new(LAVA_GRACE, now)),
        None => None,
        Some(mut m) => {
            m.advance(now);
            m.draining = touching;
            if !touching {
                m.next_damage = None;
            }
            (touching || m.remaining < m.duration).then_some(m)
        }
    };
    p.misc.lava = timer;
    let burning = timer.is_some_and(|m| m.draining && m.remaining.is_zero());
    if burning && timer.is_some_and(|m| m.next_damage.is_none_or(|t| t <= now)) {
        if let Some(m) = p.misc.lava.as_mut() {
            m.next_damage = Some(now + PULSE);
        }
        let amount = 604 + rand::rng().random_range(1..=6);
        damage(map, g, EnvironmentalDamageType::Lava, amount);
        if let Some(p) = map.players.get_mut(&g)
            && p.death != Death::Alive
        {
            p.misc.lava = None;
        }
    }
}

/// The spell the liquid a player is in puts on: on while in it, off out of
/// it (and off one brought from another map).
pub fn liquid_spell(map: &mut Map, g: Guid, spell: Option<u32>) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let before = std::mem::replace(&mut p.misc.liquid_spell, spell).or_else(|| {
        p.auras
            .has_spell(SPELL_NAXX_SLIME)
            .then_some(SPELL_NAXX_SLIME)
    });
    if let Some(old) = before.filter(|&old| Some(old) != spell) {
        crate::game::map::auras::remove_spell(map, g, old, None);
    }
    if let Some(spell) = spell
        && !map
            .players
            .get(&g)
            .is_some_and(|p| p.auras.has_spell(spell))
    {
        crate::game::map::spells::cast_triggered(map, g, spell, Some(g));
    }
}
