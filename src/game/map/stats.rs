//! Health and power regeneration, experience and levels.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{Level, SMSG_LEVELUP_INFO};

use crate::game::data::items::Body;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::death::Death;
use crate::game::map::player::{Player, UNIT_FLAG_IN_COMBAT};

/// Players regenerate every two seconds.
pub const REGEN_INTERVAL: Duration = Duration::from_secs(2);
/// Mana regenerates from spirit only this long after mana was spent.
pub const FIVE_SECOND_RULE: Duration = Duration::from_secs(5);
pub const MAX_LEVEL: u8 = 60;

const CLASS_WARRIOR: u8 = 1;
const CLASS_PALADIN: u8 = 2;
const CLASS_HUNTER: u8 = 3;
const CLASS_ROGUE: u8 = 4;
const CLASS_PRIEST: u8 = 5;
const CLASS_SHAMAN: u8 = 7;
const CLASS_MAGE: u8 = 8;
const CLASS_WARLOCK: u8 = 9;
const CLASS_DRUID: u8 = 11;

/// Health a player regenerates a tick out of combat, from spirit
/// (vanilla's class formulas).
pub fn health_regen(class: u8, spirit: f32) -> f32 {
    let v = match class {
        CLASS_DRUID | CLASS_MAGE => spirit * 0.11 + 1.0,
        CLASS_HUNTER => spirit * 0.43 - 5.5,
        CLASS_PALADIN => spirit * 0.25,
        CLASS_PRIEST => spirit * 0.15 + 1.4,
        CLASS_ROGUE => spirit * 0.84 - 13.0,
        CLASS_SHAMAN => spirit * 0.28 - 3.6,
        CLASS_WARLOCK => spirit * 0.12 + 1.5,
        CLASS_WARRIOR => spirit * 1.26 - 22.6,
        _ => spirit * 0.2,
    };
    v.max(1.0)
}

/// Mana a player regenerates a tick outside the five-second rule.
pub fn mana_regen(class: u8, spirit: f32) -> f32 {
    let v = match class {
        CLASS_PRIEST | CLASS_MAGE => spirit / 4.0 + 12.5,
        CLASS_SHAMAN => spirit / 5.0 + 17.0,
        _ => spirit / 5.0 + 15.0,
    };
    v.max(0.0)
}

/// Regenerate everyone whose tick is due.
pub fn regen(map: &mut Map) {
    let now = map.now;
    for p in map.players.values_mut() {
        if p.regen_at > now {
            continue;
        }
        p.regen_at = now + REGEN_INTERVAL;
        if p.death != Death::Alive {
            continue;
        }
        regen_player(p, now);
    }
}

fn regen_player(p: &mut Player, now: Instant) {
    use crate::game::map::auras::*;
    let in_combat = p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT);
    let class = p.row.class;
    let spirit = p.fields.get_u32(UNIT_FIELD_STAT0 + 4) as f32;
    let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
    let health = p.fields.get_u32(UNIT_FIELD_HEALTH);
    let a = &p.auras;
    let stopped = a.has(AURA_INTERRUPT_REGEN);
    // In combat, health comes back only as far as auras allow.
    let combat_share = if in_combat {
        (a.sum(AURA_MOD_REGEN_DURING_COMBAT) + a.sum(AURA_MOD_HEALTH_REGEN_IN_COMBAT)) as f32
            / 100.0
    } else {
        1.0
    };
    if health < max && !stopped && combat_share > 0.0 {
        let mut add = health_regen(class, spirit) * combat_share;
        if p.stand_state() != 0 {
            // Sitting regenerates faster.
            add *= 1.5;
        }
        add *= a.product(AURA_MOD_HEALTH_REGEN_PERCENT);
        p.fields
            .set_u32(UNIT_FIELD_HEALTH, (health + add as u32).min(max));
    }
    // Health and mana an aura gives every five seconds, with no tick of
    // its own (gear's).
    let per_five = |kind: u32, misc: Option<i32>| -> f32 {
        a.effects()
            .filter(|(_, x)| {
                x.kind == kind && x.period.is_none() && misc.is_none_or(|m| x.misc == m)
            })
            .map(|(h, x)| (x.amount * h.stacks.max(1) as i32) as f32)
            .sum::<f32>()
            * REGEN_INTERVAL.as_secs_f32()
            / 5.0
    };
    let gear_health = per_five(AURA_MOD_REGEN, None);
    if gear_health > 0.0 {
        let health = p.fields.get_u32(UNIT_FIELD_HEALTH);
        p.fields
            .set_u32(UNIT_FIELD_HEALTH, (health + gear_health as u32).min(max));
    }
    match p.power_type() {
        0 => {
            let max = p.fields.get_u32(UNIT_FIELD_MAXPOWER1);
            let mana = p.fields.get_u32(UNIT_FIELD_POWER1);
            let casting = p
                .mana_spent_at
                .is_some_and(|t| now.duration_since(t) < FIVE_SECOND_RULE);
            // Within the five-second rule, only what auras let through.
            let share = if casting {
                a.sum(AURA_MOD_MANA_REGEN_INTERRUPT) as f32 / 100.0
            } else {
                1.0
            };
            let mut add = if stopped {
                0.0
            } else {
                mana_regen(class, spirit) * share
            };
            add *= a.product_misc(AURA_MOD_POWER_REGEN_PERCENT, 0);
            add += per_five(AURA_MOD_POWER_REGEN, Some(0));
            if mana < max && add > 0.0 {
                p.fields
                    .set_u32(UNIT_FIELD_POWER1, (mana + add as u32).min(max));
            }
        }
        1 => {
            // Rage decays out of combat.
            let rage = p.fields.get_u32(UNIT_FIELD_POWER1 + 1);
            if !in_combat && rage > 0 {
                p.fields
                    .set_u32(UNIT_FIELD_POWER1 + 1, rage.saturating_sub(20));
            }
        }
        3 => {
            let max = p.fields.get_u32(UNIT_FIELD_MAXPOWER1 + 3);
            let energy = p.fields.get_u32(UNIT_FIELD_POWER1 + 3);
            let add = 20.0 * a.product_misc(AURA_MOD_POWER_REGEN_PERCENT, 3);
            p.fields
                .set_u32(UNIT_FIELD_POWER1 + 3, (energy + add as u32).min(max));
        }
        _ => {}
    }
}

/// `SMSG_LOG_XPGAIN`, as mangos lays it out: a kill (type 0) names its
/// victim and goes on with the experience before the rested bonus and the
/// group's rate; anything else (type 1) ends at its type. wow_messages has
/// the two the other way round.
fn xp_gain(victim: Option<Guid>, total: u32, base: u32) -> std::sync::Arc<[u8]> {
    let mut b = Body::default();
    b.u64(victim.unwrap_or_default().guid()).u32(total);
    match victim {
        Some(_) => b.u8(0).u32(base).f32(1.0),
        None => b.u8(1),
    };
    b.frame(SMSG_LOG_XPGAIN)
}

const SMSG_LOG_XPGAIN: u16 = 0x1D0;

/// Give a player experience, levelling them up as it fills. `victim` is the
/// creature it was for, if a kill.
pub fn give_xp(map: &mut Map, g: Guid, amount: u32, victim: Option<Guid>) {
    if amount == 0 {
        return;
    }
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.level() >= MAX_LEVEL {
        return;
    }
    let now = map.now;
    // A kill draws as much again from the rested pool.
    let base = amount;
    let amount = match victim {
        Some(_) => amount + crate::game::map::rest::spend(p, amount, now),
        None => amount,
    };
    p.outbound.send_frame(xp_gain(victim, amount, base));
    let mut xp = p.fields.get_u32(PLAYER_XP) + amount;
    let mut levels = 0;
    loop {
        let level = p.level();
        let next = data.xp_to_level(level);
        if level >= MAX_LEVEL || next == 0 || xp < next {
            break;
        }
        xp -= next;
        set_level(p, &data, level + 1);
        levels += 1;
    }
    if p.level() >= MAX_LEVEL {
        xp = 0;
    }
    p.fields.set_u32(PLAYER_XP, xp);
    if levels > 0 {
        let level = p.level();
        map.realm.registry.set_level(g, level);
    }
}

/// Set a player's level: their stats for it, full health and mana, and the
/// level-up message.
pub fn set_level(p: &mut Player, data: &crate::game::data::GameData, level: u8) {
    let level = level.clamp(1, MAX_LEVEL);
    let before: Vec<u32> = (0..5)
        .map(|i| p.fields.get_u32(UNIT_FIELD_STAT0 + i))
        .collect();
    let (health_before, mana_before) = (
        p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
        p.fields.get_u32(UNIT_FIELD_MAXPOWER1),
    );
    p.fields.set_u32(UNIT_FIELD_LEVEL, u32::from(level));
    p.apply_stats(data);
    crate::game::map::skills::level_up(p);
    crate::game::map::spells::refresh_talent_points(p, data);
    p.fields
        .set_u32(PLAYER_NEXT_LEVEL_XP, data.xp_to_level(level).max(1));
    p.row.played_level = 0;
    let max_health = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
    let max_mana = p.fields.get_u32(UNIT_FIELD_MAXPOWER1);
    p.fields.set_u32(UNIT_FIELD_HEALTH, max_health);
    p.fields.set_u32(UNIT_FIELD_POWER1, max_mana);
    let gain = |i: u16| {
        p.fields
            .get_u32(UNIT_FIELD_STAT0 + i)
            .saturating_sub(before[i as usize])
    };
    p.outbound.send(&SMSG_LEVELUP_INFO {
        new_level: Level::new(level),
        health: max_health.saturating_sub(health_before),
        mana: max_mana.saturating_sub(mana_before),
        rage: 0,
        focus: 0,
        energy: 0,
        happiness: 0,
        strength: gain(0),
        agility: gain(1),
        stamina: gain(2),
        intellect: gain(3),
        spirit: gain(4),
    });
    if level >= MAX_LEVEL {
        p.fields.set_u32(PLAYER_XP, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regen_follows_the_class_formulas_and_never_goes_below_one() {
        assert!((health_regen(CLASS_WARRIOR, 40.0) - 27.8).abs() < 0.01);
        assert_eq!(health_regen(CLASS_ROGUE, 5.0), 1.0);
        assert!((mana_regen(CLASS_MAGE, 30.0) - 20.0).abs() < 0.01);
    }
}
