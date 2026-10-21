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
pub const PROFESSIONS: &[u32] = &[129, 164, 165, 171, 182, 185, 186, 197, 202, 333, 356, 393];
/// Riding (the client's `SkillLine` 762).
pub const RIDING: u32 = 762;

fn slot_of(p: &Player, skill: u32) -> Option<u16> {
    (0..SLOTS).find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == skill)
}

/// The skills the player has.
pub fn ids(p: &Player) -> Vec<u32> {
    (0..SLOTS)
        .map(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF)
        .filter(|s| *s != 0)
        .collect()
}

/// The skill's value, if the player has it.
pub fn value(p: &Player, skill: u32) -> Option<u32> {
    slot_of(p, skill).map(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3 + 1) & 0xFFFF)
}

/// The skill's value with its bonuses, temporary and permanent (mangos'
/// `GetSkillValue`); 0 if the player hasn't it.
pub fn value_with_bonuses(p: &Player, skill: u32) -> u32 {
    slot_of(p, skill).map_or(0, |i| {
        let base = p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3 + 1) & 0xFFFF;
        let bonus = p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3 + 2);
        let temporary = i32::from(bonus as u16 as i16);
        let permanent = i32::from((bonus >> 16) as u16 as i16);
        (base as i32 + temporary + permanent).max(0) as u32
    })
}

/// Each skill's bonuses into its slot's third word, apart from its value:
/// the temporary ones (`SPELL_AURA_MOD_SKILL`) in the low half, the
/// permanent (`SPELL_AURA_MOD_SKILL_TALENT`) in the high, signed, each
/// aura by its stacks, as mangos's `SetSkill` bonuses and Thistle Tea's
/// `Skills.bonuses` have them. The client shows them as the skill's "+".
pub fn set_bonuses(p: &mut Player) {
    use crate::game::map::auras::{AURA_MOD_SKILL, AURA_MOD_SKILL_TALENT};
    for i in 0..SLOTS {
        let base = PLAYER_SKILL_INFO_1_1 + i * 3;
        let skill = (p.fields.get_u32(base) & 0xFFFF) as i32;
        if skill == 0 {
            continue;
        }
        let sum = |kind: u32| {
            p.auras
                .effects()
                .filter(|(_, a)| a.kind == kind && a.misc == skill)
                .map(|(h, a)| a.amount * h.stacks.max(1) as i32)
                .sum::<i32>()
                .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 as u16
        };
        let word = u32::from(sum(AURA_MOD_SKILL)) | u32::from(sum(AURA_MOD_SKILL_TALENT)) << 16;
        p.fields.set_u32(base + 2, word);
    }
}

/// The armor skills (`SkillLine` category 8): one point, at every level
/// (mangos's `SKILL_RANGE_MONO`, Thistle Tea's `:mono`).
const ARMOR: &[u32] = &[293, 413, 414, 415, 433];

/// The weapon skills (`SkillLine` category 6) and Defense: learned at 1,
/// up to five a level, raised by use (mangos's `SKILL_RANGE_LEVEL`,
/// `UpdateSpellTrainedSkills` and `LearnDefaultSkills`; Thistle Tea's
/// `:level` skills). Dual Wield, of the same category, is `MAXIMIZED` in
/// `SkillRaceClassInfo`, and stays at its cap. (mangos makes Fist Weapons
/// a one-point skill, as its blows go by Unarmed; wvs trains it as the
/// weapon's own.)
pub const WEAPONS: &[u32] = &[
    43, 44, 45, 46, 54, 55, 95, 136, 160, 162, 172, 173, 176, 226, 228, 229, 473,
];

fn cap(p: &Player, skill: u32, step: u32) -> u32 {
    if LANGUAGES.contains(&skill) {
        300
    } else if ARMOR.contains(&skill) {
        1
    } else if PROFESSIONS.contains(&skill) || skill == RIDING {
        // Riding too has 75 a step (mangos's `SetSkillStep` from its
        // tiers), learned at its cap.
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
        // Riding's next rank is had whole, as its first (mangos's
        // `SetSkillStep` at the tier's value).
        Some(i) if skill == RIDING => (i, max),
        // A step set keeps the value had, over a lower step's cap too
        // (mangos's `SetSkillStep`).
        Some(i) => {
            let base = PLAYER_SKILL_INFO_1_1 + i * 3;
            let had = value(p, skill).unwrap_or(1);
            p.fields.set_u32(base, skill | (step << 16));
            p.fields.set_u32(base + 1, had.max(1) | (max << 16));
            return;
        }
        None => {
            let Some(free) =
                (0..SLOTS).find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) == 0)
            else {
                return;
            };
            // A weapon skill or a profession from 1; the rest at the cap.
            let start = if WEAPONS.contains(&skill) || PROFESSIONS.contains(&skill) {
                1
            } else {
                max
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

/// Every skill the player has.
pub fn known(p: &Player) -> Vec<u32> {
    (0..SLOTS)
        .map(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF)
        .filter(|s| *s != 0)
        .collect()
}

/// Set a skill's value, within its cap.
pub fn set_value(p: &mut Player, skill: u32, value: u32) {
    if let Some(i) = slot_of(p, skill) {
        let field = PLAYER_SKILL_INFO_1_1 + i * 3 + 1;
        let max = p.fields.get_u32(field) >> 16;
        p.fields.set_u32(field, value.min(max) | (max << 16));
    }
}

/// Forget a skill: its slot is cleared, and its value as loaded too, so
/// that [`saved`] doesn't keep it.
pub fn forget_skill(p: &mut Player, skill: u32) {
    if let Some(saved) = &mut p.row.state.skills {
        saved.retain(|(s, _)| *s != skill);
    }
    if let Some(slot) = slot_of(p, skill) {
        let base = PLAYER_SKILL_INFO_1_1 + slot * 3;
        p.fields.set_u32(base, 0);
        p.fields.set_u32(base + 1, 0);
        p.fields.set_u32(base + 2, 0);
    }
}

/// The skills a character's spells give, set on login: a skill effect's,
/// and the language a language spell is a line of. Language: Common (668)
/// has no skill effect; its skill (98) comes from `SkillLineAbility`, as
/// Thistle Tea's `initial_skills` takes it (learned with the spell, for the
/// character's race and class). Without the skill the client garbles even
/// its own player's speech.
///
/// Then the values saved come back over them (mangos's `_LoadSkills`, each
/// within its cap); a character saved before skills were keeps its weapon
/// skills at their caps, as it had them.
pub fn init(p: &mut Player, data: &GameData) {
    derive(p, data);
    match p.row.state.skills.clone() {
        Some(saved) => {
            for (skill, value) in saved {
                set_value(p, skill, value.max(1));
            }
        }
        None => {
            for skill in WEAPONS {
                if let Some(max) = max_value(p, *skill) {
                    set_value(p, *skill, max);
                }
            }
        }
    }
}

/// Every skill's value, to save: those with a slot, and those loaded that
/// have none (the data didn't give their slot: a world missing its data,
/// a spell it doesn't know), kept as they were rather than dropped, as
/// mangos keeps `character_skills` apart from the spells.
pub fn saved(p: &Player) -> Vec<(u32, u32)> {
    let known = known(p);
    let mut out: Vec<(u32, u32)> = known
        .iter()
        .filter_map(|&s| value(p, s).map(|v| (s, v)))
        .collect();
    if let Some(loaded) = &p.row.state.skills {
        out.extend(loaded.iter().filter(|(s, _)| !known.contains(s)).copied());
    }
    out
}

/// The level's skills at their caps: the weapon skills, Defense, and the
/// rest that rise with the level (mangos's `UpdateSkillsForLevel` under
/// `AlwaysMaxSkillForLevel`).
pub fn maximize(p: &mut Player) {
    for i in 0..SLOTS {
        let base = PLAYER_SKILL_INFO_1_1 + i * 3;
        let skill = p.fields.get_u32(base) & 0xFFFF;
        if skill == 0 || LANGUAGES.contains(&skill) || PROFESSIONS.contains(&skill) {
            continue;
        }
        let max = p.fields.get_u32(base + 1) >> 16;
        if max > 1 {
            p.fields.set_u32(base + 1, max | (max << 16));
        }
    }
}

fn derive(p: &mut Player, data: &GameData) {
    let spells = p.spells.clone();
    let (class_bit, race_bit) = (
        1u32 << (p.row.class.max(1) - 1),
        1u32 << (p.row.race.max(1) - 1),
    );
    let saved: Vec<u32> = p
        .row
        .state
        .skills
        .iter()
        .flatten()
        .map(|(s, _)| *s)
        .collect();
    for spell in spells {
        learn_from_spell(p, data, spell);
        learn_from_line(p, data, spell);
        // A weapon skill a spell learned since brought, as it was saved (a
        // character's first weapon skills, with no slot, count as at
        // their cap).
        for skill in weapon_lines(p, data, spell) {
            if saved.contains(&skill) && slot_of(p, skill).is_none() {
                learn_skill(p, skill, 1);
            }
        }
        let languages: Vec<u32> = data
            .spells
            .skill_abilities
            .get(&spell)
            .into_iter()
            .flatten()
            .filter(|a| {
                a.acquire == 2
                    && LANGUAGES.contains(&a.skill)
                    && (a.class_mask == 0 || a.class_mask & class_bit != 0)
                    && (a.race_mask == 0 || a.race_mask & race_bit != 0)
            })
            .map(|a| a.skill)
            .collect();
        for skill in languages {
            if slot_of(p, skill).is_none() {
                learn_skill(p, skill, 1);
            }
        }
    }
}

/// The language a language spell learned teaches, at 300 (mangos's
/// `addSpell`, which gives a spell's race-or-class skill as it is learned,
/// of whatever race; Thistle Tea's `Spells.learn`).
pub fn learn_language(p: &mut Player, data: &GameData, spell: u32) {
    let languages: Vec<u32> = data
        .spells
        .skill_abilities
        .get(&spell)
        .into_iter()
        .flatten()
        .filter(|a| a.acquire == 2 && LANGUAGES.contains(&a.skill))
        .map(|a| a.skill)
        .collect();
    for skill in languages {
        if slot_of(p, skill).is_none() {
            learn_skill(p, skill, 1);
        }
    }
}

/// The skills a spell's skill effects teach, each at the step its points
/// say: on login, and as the spell is learned (mangos's `addSpell` learns
/// a spell's skill with it; a profession's rank is no passive).
pub fn learn_from_spell(p: &mut Player, data: &GameData, spell: u32) {
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    for e in s
        .effects
        .iter()
        .filter(|e| e.effect == crate::game::map::spells::EFFECT_SKILL)
    {
        let points = e.base_points + e.base_dice.max(1);
        learn_rank(p, e.misc_value.max(0) as u32, points.max(1) as u32);
    }
}

/// A rank known: its skill at its step, unless a higher rank known already
/// gave a higher one. A login reads the spells back in any order, and a
/// lower rank after a higher one changes nothing (Thistle Tea's
/// `SpellSkills.learn`); a teaching spell's step, cast, is set as it is
/// (`learn_skill`, mangos's `SetSkillStep`).
pub fn learn_rank(p: &mut Player, skill: u32, step: u32) {
    let had =
        slot_of(p, skill).map_or(0, |i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) >> 16);
    if had < step {
        learn_skill(p, skill, step);
    }
}

/// The spells `skill` gives as it is learned, for a player of `race` and
/// `class`: its `SkillLineAbility` rows learned on getting the skill
/// (acquire method 1), as mangos's `learnSkillRewardedSpells` and Thistle
/// Tea's `Skill.reward_spells`.
pub fn rewards(data: &GameData, skill: u32, race: u8, class: u8) -> Vec<u32> {
    if skill == 0 {
        return Vec::new();
    }
    let (class_bit, race_bit) = (1u32 << (class.max(1) - 1), 1u32 << (race.max(1) - 1));
    let mut out: Vec<u32> = data
        .spells
        .skill_abilities
        .values()
        .flatten()
        .filter(|a| {
            a.skill == skill
                && a.acquire == 1
                && (a.class_mask == 0 || a.class_mask & class_bit != 0)
                && (a.race_mask == 0 || a.race_mask & race_bit != 0)
        })
        .map(|a| a.spell)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Lockpicking and Poisons: skills a rogue's spell of their line brings
/// (Pick Lock, Poisons), with no skill effect, as mangos's `addSpell`'s
/// special cases have it; from 1, as they rise with use.
const TRAINED_BY_USE: [u32; 2] = [40, 633];

/// The skills `spell`'s `SkillLineAbility` rows bring a player of their
/// race and class, though it has no skill effect: Lockpicking with Pick
/// Lock, Poisons with Poisons (Thistle Tea's `initial_skills` of a rogue's
/// base abilities), each from 1 up to five a level.
pub fn learn_from_line(p: &mut Player, data: &GameData, spell: u32) {
    let (class_bit, race_bit) = (
        1u32 << (p.row.class.max(1) - 1),
        1u32 << (p.row.race.max(1) - 1),
    );
    let skills: Vec<u32> = data
        .spells
        .skill_abilities
        .get(&spell)
        .into_iter()
        .flatten()
        .filter(|a| {
            TRAINED_BY_USE.contains(&a.skill)
                && a.trivial_high == 0
                && (a.class_mask == 0 || a.class_mask & class_bit != 0)
                && (a.race_mask == 0 || a.race_mask & race_bit != 0)
        })
        .map(|a| a.skill)
        .collect();
    for skill in skills {
        if slot_of(p, skill).is_none() {
            learn_skill(p, skill, 1);
            set_value(p, skill, 1);
        }
    }
}

/// A weapon's spell learned brings its weapon skill, from 1, or from what
/// it had reached when a reset took it, if the level's cap allows
/// (mangos's `UpdateSpellTrainedSkills` for a race or class skill's
/// ability, `m_forgottenSkills`): Two-Handed Axes from a shaman's talent.
pub fn learn_weapon_lines(p: &mut Player, data: &GameData, spell: u32) {
    for skill in weapon_lines(p, data, spell) {
        if slot_of(p, skill).is_some() {
            continue;
        }
        learn_skill(p, skill, 1);
        let kept = p
            .row
            .state
            .forgotten_skills
            .iter()
            .position(|(s, _)| *s == skill);
        let value = match kept {
            Some(i) => p.row.state.forgotten_skills.remove(i).1,
            None => 1,
        };
        let fits = max_value(p, skill).is_some_and(|max| value <= max);
        set_value(p, skill, if fits { value } else { 1 });
    }
}

/// The weapon skills `spell` is the ability of for `p`'s race and class,
/// learned with it (`SkillLineAbility`'s acquire method 2, mangos's
/// `ABILITY_LEARNED_ON_GET_RACE_OR_CLASS_SKILL`).
fn weapon_lines(p: &Player, data: &GameData, spell: u32) -> Vec<u32> {
    let (class_bit, race_bit) = (
        1u32 << (p.row.class.max(1) - 1),
        1u32 << (p.row.race.max(1) - 1),
    );
    data.spells
        .skill_abilities
        .get(&spell)
        .into_iter()
        .flatten()
        .filter(|a| {
            a.acquire == 2
                && WEAPONS.contains(&a.skill)
                && (a.class_mask == 0 || a.class_mask & class_bit != 0)
                && (a.race_mask == 0 || a.race_mask & race_bit != 0)
        })
        .map(|a| a.skill)
        .collect()
}

/// A spell forgotten: the weapon skills it was the ability of go, their
/// values kept for a retrain (mangos's `UpdateSpellTrainedSkills` on
/// removal, `m_forgottenSkills`, highest kept).
pub fn forget_from_line(p: &mut Player, data: &GameData, spell: u32) {
    for skill in weapon_lines(p, data, spell) {
        let Some(value) = value(p, skill) else {
            continue;
        };
        let kept = &mut p.row.state.forgotten_skills;
        match kept.iter_mut().find(|(s, _)| *s == skill) {
            Some((_, v)) => *v = (*v).max(value),
            None => kept.push((skill, value)),
        }
        forget_skill(p, skill);
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
    if p.skills_maxed {
        maximize(p);
    }
}
