//! Spells, from vmangos's `spell_template`, with their cast times,
//! durations, ranges and radii from the DBCs.

use ahash::AHashMap;

/// One of a spell's three effects.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
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
    /// `EffectItemType`: an item (one created, one a death gives), or the
    /// family flags a modifier or trigger touches, all 64 bits of them.
    pub item_type: u64,
    pub misc_value: i32,
    pub trigger_spell: u32,
    pub points_per_combo_point: f32,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
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
    pub equipped_item_subclass_mask: i32,
    pub equipped_item_inventory_type_mask: i32,
    pub stances_not: u32,
    pub target_creature_type: u32,
    pub requires_spell_focus: u32,
    pub caster_aura_state: u32,
    pub target_aura_state: u32,
    pub mana_per_second_per_level: u32,
    pub totems: [u32; 2],
    pub max_affected_targets: u32,
    pub max_target_level: u32,
    pub spell_icon: u32,
    pub custom_flags: u32,
    /// vmangos's script for the spell, if any.
    pub script_name: String,
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
/// vmangos's `customFlags` on `spell_template`.
pub const CUSTOM_ALLOW_STACK_BETWEEN_CASTERS: u32 = 0x001;
pub const CUSTOM_NEGATIVE: u32 = 0x002;
pub const CUSTOM_POSITIVE: u32 = 0x004;
pub const CUSTOM_FIXED_DAMAGE: u32 = 0x010;
pub const CUSTOM_IGNORE_ARMOR: u32 = 0x020;
pub const CUSTOM_FACE_TARGET: u32 = 0x080;
pub const CUSTOM_SINGLE_TARGET_AURA: u32 = 0x100;
pub const CUSTOM_NOT_REMOVED_ON_EVADE: u32 = 0x400;
pub const CUSTOM_SEPARATE_AURA_PER_CASTER: u32 = 0x1000;
/// `Attributes`: it strikes on the next melee swing.
pub const ATTR_ON_NEXT_SWING: u32 = 0x4;
/// `AttributesEx`: it is channeled.
pub const ATTR_EX_CHANNELED: u32 = 0x4 | 0x40;
/// `Attributes` (word 0).
pub const ATTR_RANGED: u32 = 0x2;
pub const ATTR_ON_NEXT_SWING_2: u32 = 0x400;
pub const ATTR_ABILITY: u32 = 0x10;
pub const ATTR_TRADESKILL: u32 = 0x20;
pub const ATTR_HIDDEN: u32 = 0x80;
pub const ATTR_ONLY_INDOORS: u32 = 0x4000;
pub const ATTR_ONLY_OUTDOORS: u32 = 0x8000;
pub const ATTR_NOT_SHAPESHIFTED: u32 = 0x1_0000;
pub const ATTR_ONLY_STEALTHED: u32 = 0x2_0000;
pub const ATTR_LEVEL_DAMAGE_CALCULATION: u32 = 0x8_0000;
pub const ATTR_CANCELS_AUTO_ATTACK: u32 = 0x10_0000;
pub const ATTR_CASTABLE_WHILE_DEAD: u32 = 0x80_0000;
pub const ATTR_ALLOW_WHILE_MOUNTED: u32 = 0x100_0000;
pub const ATTR_DEBUFF: u32 = 0x400_0000;
pub const ATTR_ALLOW_WHILE_SITTING: u32 = 0x800_0000;
pub const ATTR_NOT_IN_COMBAT: u32 = 0x1000_0000;
pub const ATTR_NO_IMMUNITIES: u32 = 0x2000_0000;
pub const ATTR_CANT_CANCEL: u32 = 0x8000_0000;
/// `AttributesEx` (word 1).
pub const ATTR_EX_USE_ALL_MANA: u32 = 0x2;
pub const ATTR_EX_ALLOW_WHILE_STEALTHED: u32 = 0x20;
pub const ATTR_EX_NO_REFLECTION: u32 = 0x80;
pub const ATTR_EX_ONLY_PEACEFUL_TARGETS: u32 = 0x100;
pub const ATTR_EX_FROM_BEHIND: u32 = 0x200;
pub const ATTR_EX_NO_THREAT: u32 = 0x400;
pub const ATTR_EX_FAILURE_BREAKS_STEALTH: u32 = 0x1000;
/// Its aura holds the bearer's body (`UNIT_FLAG_PREVENT_ANIM`): stuns,
/// Polymorph, Sap, Sleep, Feign Death.
pub const ATTR_EX_PREVENTS_ANIM: u32 = 0x4_0000;
pub const ATTR_EX_DISCOUNT_POWER_ON_MISS: u32 = 0x800_0000;
pub const ATTR_EX_REQ_COMBO_POINTS: u32 = 0x10_0000 | 0x40_0000;
/// `AttributesEx2` (word 2).
pub const ATTR_EX2_ALLOW_DEAD_TARGET: u32 = 0x1;
pub const ATTR_EX2_IGNORE_LOS: u32 = 0x4;
pub const ATTR_EX2_AUTO_REPEAT: u32 = 0x20;
pub const ATTR_EX2_ALLOW_WHILE_INVISIBLE: u32 = 0x4000;
pub const ATTR_EX2_BEHIND_TARGET: u32 = 0x10_0000;
pub const ATTR_EX2_NO_SCHOOL_IMMUNITIES: u32 = 0x400_0000;
pub const ATTR_EX2_NOT_AN_ACTION: u32 = 0x1000_0000;
pub const ATTR_EX2_CANT_CRIT: u32 = 0x2000_0000;
/// `AttributesEx3` (word 3).
/// A resurrection that doesn't wait out the reclaim delay (Rebirth's).
/// `SPELL_ATTR_EX3_NO_PROC_EQUIP_REQUIREMENT`: its procs ask for no gear.
pub const ATTR_EX3_NO_PROC_EQUIP_REQUIREMENT: u32 = 0x2;
/// `SPELL_ATTR_EX3_COMPLETELY_BLOCKED`: a block takes all of it, as a miss.
pub const ATTR_EX3_COMPLETELY_BLOCKED: u32 = 0x8;
pub const ATTR_EX3_NO_RES_TIMER: u32 = 0x10;
/// A death it causes wears no gear down.
pub const ATTR_EX3_NO_DURABILITY_LOSS: u32 = 0x20;
pub const ATTR_EX3_DOT_STACKING: u32 = 0x80;
pub const ATTR_EX3_BATTLEGROUND: u32 = 0x800;
pub const ATTR_EX3_NOT_A_PROC: u32 = 0x200;
pub const ATTR_EX3_SUPPRESS_CASTER_PROCS: u32 = 0x1_0000;
pub const ATTR_EX3_SUPPRESS_TARGET_PROCS: u32 = 0x2_0000;
pub const ATTR_EX3_ALWAYS_HIT: u32 = 0x4_0000;
pub const ATTR_EX3_DEATH_PERSISTENT: u32 = 0x10_0000;
pub const ATTR_EX3_REQ_OFFHAND: u32 = 0x100_0000;
pub const ATTR_EX3_CAN_PROC_FROM_PROCS: u32 = 0x400_0000;
pub const ATTR_EX3_IGNORE_CASTER_AND_TARGET_RESTRICTIONS: u32 = 0x1000_0000;
pub const ATTR_EX3_IGNORE_CASTER_MODIFIERS: u32 = 0x2000_0000;
/// `AttributesEx4` (word 4): the target's damage taken modifiers don't
/// count (Decimate).
pub const ATTR_EX4_IGNORE_DAMAGE_TAKEN_MODIFIERS: u32 = 0x100;
/// No threat for the help it gives (a heal's, a buff's).
pub const ATTR_EX4_NO_HELPFUL_THREAT: u32 = 0x8;
/// No threat when it first lands.
pub const ATTR_EX2_NO_INITIAL_THREAT: u32 = 0x40_0000;
/// It may be cast while another spell is channelled, or channelled while
/// another is cast (mangos's `SPELL_ATTR_EX4_ALLOW_CAST_WHILE_CASTING`).
pub const ATTR_EX4_ALLOW_CAST_WHILE_CASTING: u32 = 0x80;
/// A target-trigger aura of it rolls only for its owner among a cast's
/// recipients (Relentless Strikes, at the finisher's dummy on the rogue;
/// mangos's `SPELL_ATTR_EX4_CLASS_TRIGGER_ONLY_ON_TARGET`).
pub const ATTR_EX4_CLASS_TRIGGER_ONLY_ON_TARGET: u32 = 0x2;

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

    pub fn has_aura(&self, aura: u32) -> bool {
        self.effects
            .iter()
            .any(|e| matches!(e.effect, 6 | 27 | 35) && e.aura == aura)
    }

    pub fn attr(&self, word: usize, bit: u32) -> bool {
        self.attributes[word] & bit != 0
    }

    /// One of vmangos's `customFlags`.
    pub fn custom(&self, flag: u32) -> bool {
        self.custom_flags & flag != 0
    }

    /// A physical ability: melee or ranged, with a weapon.
    pub fn ability(&self) -> bool {
        self.attr(0, ATTR_ABILITY)
    }

    pub fn auto_repeat(&self) -> bool {
        self.attr(0, ATTR_RANGED) && self.attr(2, ATTR_EX2_AUTO_REPEAT)
    }

    /// From behind the target only (Backstab, Ambush).
    pub fn from_behind(&self) -> bool {
        self.attributes[2] == ATTR_EX2_BEHIND_TARGET && self.attr(1, ATTR_EX_FROM_BEHIND)
    }

    pub fn on_next_swing(&self) -> bool {
        self.attr(0, ATTR_ON_NEXT_SWING | ATTR_ON_NEXT_SWING_2)
    }

    /// The school as a mask: physical 1, holy 2, and so on.
    pub fn school_mask(&self) -> u32 {
        1 << self.school.min(6)
    }

    /// A finishing move: it spends combo points.
    pub fn finisher(&self) -> bool {
        self.attr(1, ATTR_EX_REQ_COMBO_POINTS)
            || self.effects.iter().any(|e| e.points_per_combo_point != 0.0)
    }

    /// Damage class: 0 none, 1 magic, 2 melee, 3 ranged.
    pub fn melee_or_ranged(&self) -> bool {
        matches!(self.dmg_class, 2 | 3)
    }
}

/// `spell_proc_event`: when a proc aura goes off, over the spell's own.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct ProcEvent {
    pub school_mask: u32,
    pub family: u32,
    /// For each of the aura's effects.
    pub family_mask: [u64; 3],
    pub flags: u32,
    pub ex: u32,
    pub ppm: f32,
    pub chance: f32,
    pub cooldown: u32,
}

/// `spell_chain`: a spell's rank among its kind.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct SpellRank {
    pub prev: u32,
    pub first: u32,
    pub rank: u32,
    /// A spell needed before this one (`req_spell`).
    pub req: u32,
}

/// `SpellItemEnchantment`: up to three effects an enchantment gives.
#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct Enchantment {
    pub id: u32,
    pub effects: [(u32, i32, i32, u32); 3],
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct SpellDuration {
    pub base: i32,
    pub per_level: i32,
    pub max: i32,
}

#[derive(Debug, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
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
    /// `TalentTab`: tab → the classes (a mask) whose tree it is.
    pub talent_tab_classes: AHashMap<u32, u32>,
    /// A talent's rank or a rank trained after it (Mortal Strike's from the
    /// trainer, by the skill line's `superseded_by`) → the talent.
    pub talent_lines: AHashMap<u32, u32>,
    /// A rank in a talent's line → the rank before it there (Mortal
    /// Strike's second → the talent's own), as mangos's spell chain links
    /// them from the skill line and vmangos's `spell_chain` doesn't.
    pub talent_prevs: AHashMap<u32, u32>,
    /// `creature_spells`: list → its spells.
    pub creature_spells: AHashMap<u32, Vec<CreatureSpell>>,
    /// Spells by the skill line they teach (`SkillLineAbility`): spell → skill.
    pub spell_skills: AHashMap<u32, u32>,
    pub skill_abilities: AHashMap<u32, Vec<SkillAbility>>,
    /// `SkillRaceClassInfo`: skill → (race mask, class mask, flags).
    pub skill_race_class: AHashMap<u32, Vec<(u32, u32, u32)>>,
    pub proc_events: AHashMap<u32, ProcEvent>,
    pub ranks: AHashMap<u32, SpellRank>,
    /// `spell_threat`: flat threat and a multiplier.
    pub threat: AHashMap<u32, (i32, f32)>,
    /// `spell_group`: groups a spell is in, and each group's stacking rule.
    pub groups: AHashMap<u32, Vec<u32>>,
    pub group_rules: AHashMap<u32, u32>,
    /// A spell's place in a group (`group_spell_id`), (group, spell) to it:
    /// the order a group's rule 3 keeps the highest of.
    pub group_orders: AHashMap<(u32, u32), u32>,
    pub enchantments: AHashMap<u32, Enchantment>,
    /// `spell_elixir`: battle (1) or guardian (2) elixirs, and flasks (3).
    pub elixirs: AHashMap<u32, u32>,
    /// `spell_proc_item_enchant`: procs a minute.
    pub enchant_ppm: AHashMap<u32, f32>,
    /// `spell_learn_spell`: spells another brings.
    pub learn_with: AHashMap<u32, Vec<u32>>,
    /// `spell_cone`: a cone's width in degrees, where it isn't 60.
    pub cones: AHashMap<u32, i32>,
    /// `spell_script_target`: the creatures (type 1) or objects (type 0)
    /// a scripted spell aims at.
    pub script_targets: AHashMap<u32, Vec<(u32, u32)>>,
    /// The condition a script target must meet, by (spell, type, entry):
    /// the caster the source, the candidate the target.
    pub script_target_conditions: AHashMap<(u32, u32, u32), u32>,
}

/// `SkillLineAbility`: a spell in a skill line, and the ranks it greys at.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct SkillAbility {
    pub skill: u32,
    pub spell: u32,
    pub race_mask: u32,
    pub class_mask: u32,
    pub min_rank: u32,
    pub superseded_by: u32,
    pub acquire: u32,
    pub trivial_high: u32,
    pub trivial_low: u32,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct TalentInfo {
    pub id: u32,
    pub tab: u32,
    pub row: u32,
    pub column: u32,
    pub ranks: [u32; 5],
    pub depends_on: u32,
    pub depends_rank: u32,
}

/// A creature's spell from its list: its cast target type and that
/// target's parameters, its cast flags, its delays (in milliseconds, from
/// the table's seconds), and the `creature_spells_scripts` its cast starts.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct CreatureSpell {
    pub spell: u32,
    pub probability: u32,
    pub target: u32,
    pub target_param1: u32,
    pub target_param2: u32,
    pub flags: u32,
    pub delay_initial: (u32, u32),
    pub delay_repeat: (u32, u32),
    pub script: u32,
}

impl SpellData {
    pub fn get(&self, id: u32) -> Option<&SpellTemplate> {
        self.spells.get(&id)
    }

    /// `spell`'s place in `group`, for its rule 3 (the highest kept): its
    /// `group_spell_id`, else its id.
    pub fn group_order(&self, group: u32, spell: u32) -> u32 {
        self.group_orders
            .get(&(group, spell))
            .copied()
            .unwrap_or(spell)
    }

    /// The first rank of the spell's kind.
    pub fn first_rank(&self, id: u32) -> u32 {
        self.ranks
            .get(&id)
            .map(|r| r.first)
            .filter(|f| *f != 0)
            .unwrap_or(id)
    }

    /// Whether `a` and `b` are ranks of one spell.
    pub fn same_kind(&self, a: u32, b: u32) -> bool {
        a == b || self.first_rank(a) == self.first_rank(b)
    }

    /// A higher rank with no `spell_proc_event` row of its own takes its
    /// first rank's, by `spell_chain` or a talent's ranks (vmangos's
    /// `LoadSpellProcEvents` over its chains, Thistle Tea's `proc_rule`):
    /// Lightning Shield's, the seals', Flurry's and Enrage's higher ranks.
    pub fn inherit_proc_events(&mut self) {
        let mut firsts: Vec<(u32, u32)> = self.ranks.iter().map(|(id, r)| (*id, r.first)).collect();
        for t in self.talents.values() {
            firsts.extend(
                t.ranks[1..]
                    .iter()
                    .filter(|r| **r != 0)
                    .map(|r| (*r, t.ranks[0])),
            );
        }
        for (id, first) in firsts {
            if id != first
                && !self.proc_events.contains_key(&id)
                && let Some(p) = self.proc_events.get(&first).copied()
            {
                self.proc_events.insert(id, p);
            }
        }
    }

    /// A later rank with no `spell_group` row of its own is in its first
    /// rank's groups, at its place (vmangos's and Thistle Tea's rank-root
    /// membership): Blessing of Might's ranks beside Greater Blessing of
    /// Might's.
    pub fn inherit_groups(&mut self) {
        let firsts: Vec<(u32, u32)> = self
            .ranks
            .iter()
            .filter(|(id, r)| r.first != 0 && r.first != **id)
            .map(|(id, r)| (*id, r.first))
            .collect();
        for (id, first) in firsts {
            if self.groups.contains_key(&id) {
                continue;
            }
            let Some(groups) = self.groups.get(&first).cloned() else {
                continue;
            };
            for g in &groups {
                let order = self.group_order(*g, first);
                self.group_orders.insert((*g, id), order);
            }
            self.groups.insert(id, groups);
        }
    }

    /// Index each talent's ranks, and the ranks a trainer teaches after
    /// them, by talent (Thistle Tea's `Talent.by_spell` through the chain;
    /// what mangos's `resetTalents` takes with the talent), and each such
    /// rank's rank before.
    pub fn index_talent_lines(&mut self) {
        let mut lines = AHashMap::new();
        let mut prevs = AHashMap::new();
        for t in self.talents.values() {
            for r in t.ranks.iter().filter(|r| **r != 0) {
                let mut at = *r;
                for _ in 0..16 {
                    lines.insert(at, t.id);
                    let next = self
                        .skill_abilities
                        .get(&at)
                        .and_then(|a| a.iter().find(|a| a.superseded_by != 0))
                        .map(|a| a.superseded_by);
                    match next {
                        Some(n) if !lines.contains_key(&n) => {
                            prevs.insert(n, at);
                            at = n;
                        }
                        _ => break,
                    }
                }
            }
        }
        self.talent_lines = lines;
        self.talent_prevs = prevs;
    }

    /// The rank before `spell` in its talent's line: the talent's own
    /// before its first trained rank.
    pub fn talent_prev(&self, spell: u32) -> Option<u32> {
        self.talent_prevs.get(&spell).copied()
    }

    /// The talent `spell` is a rank of, or was trained after.
    pub fn talent_of(&self, spell: u32) -> Option<u32> {
        self.talent_lines.get(&spell).copied()
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
