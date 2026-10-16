//! Spells, from vmangos's `spell_template`, with their cast times,
//! durations, ranges and radii from the DBCs.

use ahash::AHashMap;

/// One of a spell's three effects.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpellEffect {
    pub effect: u32,
    pub die_sides: i32,
    pub base_dice: i32,
    pub dice_per_level: f32,
    pub real_points_per_level: f32,
    pub base_points: i32,
    pub bonus_coefficient: f32,
    pub mechanic: u32,
    pub target_a: u32,
    pub target_b: u32,
    pub radius_index: u32,
    pub aura: u32,
    pub amplitude: u32,
    pub multiple_value: f32,
    pub chain_target: u32,
    pub item_type: u32,
    pub misc_value: i32,
    pub trigger_spell: u32,
    pub points_per_combo_point: f32,
}

#[derive(Debug, Clone, Default)]
pub struct SpellTemplate {
    pub id: u32,
    pub school: u32,
    pub category: u32,
    pub dispel: u32,
    pub mechanic: u32,
    pub attributes: [u32; 5],
    pub stances: u32,
    pub targets: u32,
    pub casting_time_index: u32,
    pub recovery_time: u32,
    pub category_recovery_time: u32,
    pub interrupt_flags: u32,
    pub aura_interrupt_flags: u32,
    pub channel_interrupt_flags: u32,
    pub proc_flags: u32,
    pub proc_chance: u32,
    pub proc_charges: u32,
    pub max_level: u32,
    pub base_level: u32,
    pub spell_level: u32,
    pub duration_index: u32,
    pub power_type: u32,
    pub mana_cost: u32,
    pub mana_cost_per_level: u32,
    pub mana_per_second: u32,
    pub mana_cost_percentage: u32,
    pub range_index: u32,
    pub speed: f32,
    pub stack_amount: u32,
    pub reagents: [(u32, u32); 8],
    pub equipped_item_class: i32,
    pub effects: [SpellEffect; 3],
    pub spell_visual: u32,
    pub name: String,
    pub rank: String,
    pub start_recovery_category: u32,
    pub start_recovery_time: u32,
    pub family_name: u32,
    pub family_flags: u64,
    pub dmg_class: u32,
    pub prevention_type: u32,
    pub dmg_multiplier: [f32; 3],
}

/// `Attributes`: the spell is a passive one.
pub const ATTR_PASSIVE: u32 = 0x40;
/// `Attributes`: it strikes on the next melee swing.
pub const ATTR_ON_NEXT_SWING: u32 = 0x4;
/// `AttributesEx`: it is channeled.
pub const ATTR_EX_CHANNELED: u32 = 0x4 | 0x40;

impl SpellTemplate {
    pub fn passive(&self) -> bool {
        self.attributes[0] & ATTR_PASSIVE != 0
    }

    pub fn channeled(&self) -> bool {
        self.attributes[1] & ATTR_EX_CHANNELED != 0
    }

    pub fn has_effect(&self, effect: u32) -> bool {
        self.effects.iter().any(|e| e.effect == effect)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SpellDuration {
    pub base: i32,
    pub per_level: i32,
    pub max: i32,
}

#[derive(Debug, Default)]
pub struct SpellData {
    pub spells: AHashMap<u32, SpellTemplate>,
    /// `SpellCastTimes`: index → base milliseconds.
    pub cast_times: AHashMap<u32, i32>,
    pub durations: AHashMap<u32, SpellDuration>,
    /// `SpellRange`: index → (min, max) yards.
    pub ranges: AHashMap<u32, (f32, f32)>,
    /// `SpellRadius`: index → yards.
    pub radii: AHashMap<u32, f32>,
    /// `spell_target_position`: where a teleport spell sends.
    pub target_positions: AHashMap<u32, (u32, f32, f32, f32, f32)>,
    /// `Talent`: talent id → its ranks' spells.
    pub talents: AHashMap<u32, TalentInfo>,
    /// `creature_spells`: list → its spells.
    pub creature_spells: AHashMap<u32, Vec<CreatureSpell>>,
    /// Spells by the skill line they teach (`SkillLineAbility`): spell → skill.
    pub spell_skills: AHashMap<u32, u32>,
}

#[derive(Debug, Clone, Default)]
pub struct TalentInfo {
    pub id: u32,
    pub tab: u32,
    pub row: u32,
    pub column: u32,
    pub ranks: [u32; 5],
    pub depends_on: u32,
    pub depends_rank: u32,
}

/// A creature's spell from its list.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreatureSpell {
    pub spell: u32,
    pub probability: u32,
    pub target: u32,
    pub delay_initial: (u32, u32),
    pub delay_repeat: (u32, u32),
}

impl SpellData {
    pub fn get(&self, id: u32) -> Option<&SpellTemplate> {
        self.spells.get(&id)
    }

    pub fn cast_time(&self, s: &SpellTemplate) -> u32 {
        self.cast_times
            .get(&s.casting_time_index)
            .copied()
            .unwrap_or(0)
            .max(0) as u32
    }

    /// How long the spell's aura lasts at `level`, in milliseconds; `None`
    /// for forever.
    pub fn duration(&self, s: &SpellTemplate, level: u32) -> Option<u32> {
        let d = self.durations.get(&s.duration_index)?;
        if d.base < 0 {
            return None;
        }
        let level_bonus = d.per_level * (level.saturating_sub(s.base_level) as i32);
        let mut ms = d.base + level_bonus;
        if d.max > 0 {
            ms = ms.min(d.max);
        }
        Some(ms.max(0) as u32)
    }

    pub fn range(&self, s: &SpellTemplate) -> (f32, f32) {
        self.ranges
            .get(&s.range_index)
            .copied()
            .unwrap_or((0.0, 5.0))
    }

    pub fn radius(&self, index: u32) -> f32 {
        self.radii.get(&index).copied().unwrap_or(0.0)
    }
}
