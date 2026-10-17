//! Skills: the 128 `PLAYER_SKILL_INFO` slots, three words each (the skill
//! and its step, the value and its cap, the bonus). Spells with a skill
//! effect teach them; weapon and class skills cap at five per level.

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::map::player::Player;

const SLOTS: u16 = 128;

const LANGUAGES: &[u32] = &[
    98, 109, 111, 113, 115, 137, 138, 139, 140, 141, 313, 315, 673,
];
const PROFESSIONS: &[u32] = &[129, 164, 165, 171, 182, 185, 186, 197, 202, 333, 356, 393];

fn slot_of(p: &Player, skill: u32) -> Option<u16> {
    (0..SLOTS).find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == skill)
}

/// The skill's value, if the player has it.
pub fn value(p: &Player, skill: u32) -> Option<u32> {
    slot_of(p, skill).map(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3 + 1) & 0xFFFF)
}

fn cap(p: &Player, skill: u32, step: u32) -> u32 {
    if LANGUAGES.contains(&skill) {
        300
    } else if PROFESSIONS.contains(&skill) {
        75 * step.max(1)
    } else {
        5 * u32::from(p.level()).max(1)
    }
}

/// Learn `skill` at `step`, or raise its step.
pub fn learn_skill(p: &mut Player, skill: u32, step: u32) {
    if skill == 0 {
        return;
    }
    let max = cap(p, skill, step);
    let (slot, value) = match slot_of(p, skill) {
        Some(i) => (i, value(p, skill).unwrap_or(1)),
        None => {
            let Some(free) =
                (0..SLOTS).find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) == 0)
            else {
                return;
            };
            let start = if LANGUAGES.contains(&skill) || !PROFESSIONS.contains(&skill) {
                max
            } else {
                1
            };
            (free, start)
        }
    };
    let base = PLAYER_SKILL_INFO_1_1 + slot * 3;
    p.fields.set_u32(base, skill | (step << 16));
    p.fields.set_u32(base + 1, value.min(max) | (max << 16));
}

/// The skill's cap, if the player has it.
pub fn max_value(p: &Player, skill: u32) -> Option<u32> {
    slot_of(p, skill).map(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3 + 1) >> 16)
}

/// Raise a skill by `by`, up to its cap: whether it rose.
pub fn raise(p: &mut Player, skill: u32, by: u32) -> bool {
    let Some(i) = slot_of(p, skill) else {
        return false;
    };
    let field = PLAYER_SKILL_INFO_1_1 + i * 3 + 1;
    let word = p.fields.get_u32(field);
    let (value, max) = (word & 0xFFFF, word >> 16);
    if value >= max {
        return false;
    }
    p.fields.set_u32(field, (value + by).min(max) | (max << 16));
    true
}

/// Forget a skill: its slot is cleared.
pub fn forget_skill(p: &mut Player, skill: u32) {
    if let Some(slot) = slot_of(p, skill) {
        let base = PLAYER_SKILL_INFO_1_1 + slot * 3;
        p.fields.set_u32(base, 0);
        p.fields.set_u32(base + 1, 0);
        p.fields.set_u32(base + 2, 0);
    }
}

/// The skills a character's spells give, set on login.
pub fn init(p: &mut Player, data: &GameData) {
    let spells = p.spells.clone();
    for spell in spells {
        let Some(s) = data.spells.get(spell) else {
            continue;
        };
        for e in s
            .effects
            .iter()
            .filter(|e| e.effect == crate::game::map::spells::EFFECT_SKILL)
        {
            let points = e.base_points + e.base_dice.max(1);
            learn_skill(p, e.misc_value.max(0) as u32, points.max(1) as u32);
        }
    }
}

/// On a new level, the level-capped skills rise with it.
pub fn level_up(p: &mut Player) {
    for i in 0..SLOTS {
        let base = PLAYER_SKILL_INFO_1_1 + i * 3;
        let id = p.fields.get_u32(base);
        let skill = id & 0xFFFF;
        if skill == 0 || LANGUAGES.contains(&skill) || PROFESSIONS.contains(&skill) {
            continue;
        }
        let max = cap(p, skill, id >> 16);
        let value = p.fields.get_u32(base + 1) & 0xFFFF;
        p.fields
            .set_u32(base + 1, value.max(1).min(max) | (max << 16));
    }
}
