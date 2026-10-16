//! Health and power regeneration, experience and levels.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    Level, SMSG_LEVELUP_INFO, SMSG_LOG_XPGAIN, SMSG_LOG_XPGAIN_ExperienceAwardType,
};

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
    let in_combat = p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT);
    let class = p.row.class;
    let spirit = p.fields.get_u32(UNIT_FIELD_STAT0 + 4) as f32;
    let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
    let health = p.fields.get_u32(UNIT_FIELD_HEALTH);
    if !in_combat && health < max {
        let mut add = health_regen(class, spirit);
        if p.stand_state() != 0 {
            // Sitting regenerates faster.
            add *= 1.5;
        }
        p.fields
            .set_u32(UNIT_FIELD_HEALTH, (health + add as u32).min(max));
    }
    match p.power_type() {
        0 => {
            let max = p.fields.get_u32(UNIT_FIELD_MAXPOWER1);
            let mana = p.fields.get_u32(UNIT_FIELD_POWER1);
            if mana < max
                && p.mana_spent_at
                    .is_none_or(|t| now.duration_since(t) >= FIVE_SECOND_RULE)
            {
                let add = mana_regen(class, spirit) as u32;
                p.fields.set_u32(UNIT_FIELD_POWER1, (mana + add).min(max));
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
            p.fields
                .set_u32(UNIT_FIELD_POWER1 + 3, (energy + 20).min(max));
        }
        _ => {}
    }
}

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
    p.outbound.send(&SMSG_LOG_XPGAIN {
        target: victim.unwrap_or_default(),
        total_exp: amount,
        exp_type: match victim {
            Some(_) => SMSG_LOG_XPGAIN_ExperienceAwardType::Kill,
            None => SMSG_LOG_XPGAIN_ExperienceAwardType::NonKill {
                experience_without_rested: amount,
                exp_group_bonus: 1.0,
            },
        },
    });
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
