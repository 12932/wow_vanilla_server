//! The environment's harm, as Thistle Tea's `Core.Environment` has it:
//! breath under water (a minute, then a fifth of health every two
//! seconds), fatigue in the deep sea, and lava and slime burning. The
//! liquid under a player comes from the navmesh: its surface everywhere,
//! and its kind where a WMO holds it.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_MOD_WATER_BREATHING, AURA_WATER_BREATHING};
use crate::game::map::death::Death;

pub const BREATH: Duration = Duration::from_secs(60);
pub const FATIGUE: Duration = Duration::from_secs(60);
const PULSE: Duration = Duration::from_secs(2);
/// How often a player's liquid is looked at.
const CHECK_EVERY: Duration = Duration::from_millis(500);
/// How deep the head is: two yards at scale 1.
const BODY_HEIGHT: f32 = 2.0;

// The navmesh's liquid kinds.
pub const LIQUID_WATER: u8 = 1;
pub const LIQUID_OCEAN: u8 = 2;
pub const LIQUID_MAGMA: u8 = 3;
pub const LIQUID_SLIME: u8 = 4;
pub const LIQUID_NAXX_SLIME: u8 = 21;

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
        .and_then(|s| map.unit_fields(g).map(|f| f.get_u32(UNIT_FIELD_RESISTANCES + s)))
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

/// What a player's liquid is: (kind, surface), the kind 0 where the
/// navmesh doesn't say.
fn liquid_at(map: &mut Map, at: Vector3d) -> Option<(u8, f32)> {
    map.nav.as_mut()?.liquid(at)
}

/// Every player's breath, fatigue and burning.
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
        let protected = p.gm
            || p.flight.is_some()
            || p.movement.flags.get_on_transport().is_some()
            || p.death != Death::Alive;
        let at = p.position();
        let scale = p.fields.get_f32(OBJECT_FIELD_SCALE_X).max(0.1);
        let swimming = p.movement.flags.get_swimming().is_some();
        let breath_for = if p.auras.has(AURA_WATER_BREATHING) {
            Duration::ZERO
        } else {
            BREATH.mul_f32(p.auras.product(AURA_MOD_WATER_BREATHING))
        };
        if protected {
            stop(map, g);
            continue;
        }
        let liquid = if swimming || map.nav.is_some() {
            liquid_at(map, at)
        } else {
            None
        };
        let depth = liquid.map_or(0.0, |(_, surface)| surface - at.z);
        let submerged = depth > BODY_HEIGHT * scale;
        breathe(map, g, submerged && breath_for > Duration::ZERO, breath_for, depth > 0.0, now);
        let kind = liquid.map_or(0, |l| l.0);
        fatigue(map, g, depth > 0.0 && kind == LIQUID_OCEAN, now);
        if depth > 0.0 && matches!(kind, LIQUID_MAGMA | LIQUID_SLIME | LIQUID_NAXX_SLIME) {
            burn(map, g, kind, now);
        }
    }
}

fn stop(map: &mut Map, g: Guid) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.misc.breath.take().is_some() {
        p.outbound.send(&SMSG_STOP_MIRROR_TIMER {
            timer: TimerType::Breath,
        });
    }
    if p.misc.fatigue.take().is_some() {
        p.outbound.send(&SMSG_STOP_MIRROR_TIMER {
            timer: TimerType::Fatigue,
        });
    }
}

/// The breath timer: draining with the head under, filling above it,
/// gone when full or out of the water.
pub fn breathe(map: &mut Map, g: Guid, under: bool, duration: Duration, in_water: bool, now: Instant) {
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
        let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        if let Some(m) = p.misc.breath.as_mut() {
            m.next_damage = Some(now + PULSE);
        }
        damage(map, g, EnvironmentalDamageType::Drowning, (max / 5).max(1));
    }
}

/// The fatigue timer, in the deep sea.
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
        let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        if let Some(m) = p.misc.fatigue.as_mut() {
            m.next_damage = Some(now + PULSE);
        }
        damage(map, g, EnvironmentalDamageType::Exhausted, (max / 5).max(1));
    }
}

/// Lava and slime burn every two seconds.
fn burn(map: &mut Map, g: Guid, kind: u8, now: Instant) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.misc.env_damage_at.is_some_and(|t| t > now) {
        return;
    }
    p.misc.env_damage_at = Some(now + PULSE);
    let amount = rand::rng().random_range(600..=700);
    let kind = if kind == LIQUID_MAGMA {
        EnvironmentalDamageType::Lava
    } else {
        EnvironmentalDamageType::Slime
    };
    damage(map, g, kind, amount);
}
