//! Auras: a spell's lasting effects on a unit. A spell's aura effects on a
//! unit from one caster form a holder, which takes one of the unit's 48
//! visible slots (32 helpful, 16 harmful) unless it is hidden. Holders
//! stack, carry charges, and replace or block each other by vmangos's rules
//! as Thistle Tea ports them: a caster's own refresh, ranks of one spell,
//! the spell groups, and the exclusive kinds (seals, aspects, curses,
//! blessings, stings, armors, shapeshifts, mounts, food and drink).
//!
//! Periodic effects tick; stat, speed, crit and the like change the unit
//! while they last ([`crate::game::map::derive`]); roots, stuns and
//! silences hold it.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;

pub const AURA_BIND_SIGHT: u32 = 1;
pub const AURA_MOD_POSSESS: u32 = 2;
pub const AURA_PERIODIC_DAMAGE: u32 = 3;
pub const AURA_DUMMY: u32 = 4;
pub const AURA_MOD_CONFUSE: u32 = 5;
pub const AURA_MOD_CHARM: u32 = 6;
pub const AURA_MOD_FEAR: u32 = 7;
pub const AURA_PERIODIC_HEAL: u32 = 8;
pub const AURA_MOD_ATTACKSPEED: u32 = 9;
pub const AURA_MOD_THREAT: u32 = 10;
pub const AURA_MOD_TAUNT: u32 = 11;
pub const AURA_MOD_STUN: u32 = 12;
pub const AURA_MOD_DAMAGE_DONE: u32 = 13;
pub const AURA_MOD_DAMAGE_TAKEN: u32 = 14;
pub const AURA_DAMAGE_SHIELD: u32 = 15;
pub const AURA_MOD_STEALTH: u32 = 16;
pub const AURA_MOD_STEALTH_DETECT: u32 = 17;
pub const AURA_MOD_INVISIBILITY: u32 = 18;
pub const AURA_MOD_INVISIBILITY_DETECTION: u32 = 19;
pub const AURA_OBS_MOD_HEALTH: u32 = 20;
pub const AURA_OBS_MOD_MANA: u32 = 21;
pub const AURA_MOD_RESISTANCE: u32 = 22;
pub const AURA_PERIODIC_TRIGGER_SPELL: u32 = 23;
pub const AURA_PERIODIC_ENERGIZE: u32 = 24;
pub const AURA_MOD_PACIFY: u32 = 25;
pub const AURA_MOD_ROOT: u32 = 26;
pub const AURA_MOD_SILENCE: u32 = 27;
pub const AURA_REFLECT_SPELLS: u32 = 28;
pub const AURA_MOD_STAT: u32 = 29;
pub const AURA_MOD_SKILL: u32 = 30;
pub const AURA_MOD_INCREASE_SPEED: u32 = 31;
pub const AURA_MOD_INCREASE_MOUNTED_SPEED: u32 = 32;
pub const AURA_MOD_DECREASE_SPEED: u32 = 33;
pub const AURA_MOD_INCREASE_HEALTH: u32 = 34;
pub const AURA_MOD_INCREASE_ENERGY: u32 = 35;
pub const AURA_MOD_SHAPESHIFT: u32 = 36;
pub const AURA_EFFECT_IMMUNITY: u32 = 37;
pub const AURA_STATE_IMMUNITY: u32 = 38;
pub const AURA_SCHOOL_IMMUNITY: u32 = 39;
pub const AURA_DAMAGE_IMMUNITY: u32 = 40;
pub const AURA_DISPEL_IMMUNITY: u32 = 41;
pub const AURA_PROC_TRIGGER_SPELL: u32 = 42;
pub const AURA_PROC_TRIGGER_DAMAGE: u32 = 43;
pub const AURA_TRACK_CREATURES: u32 = 44;
pub const AURA_TRACK_RESOURCES: u32 = 45;
pub const AURA_MOD_PARRY_PERCENT: u32 = 47;
pub const AURA_MOD_DODGE_PERCENT: u32 = 49;
pub const AURA_MOD_BLOCK_SKILL: u32 = 50;
pub const AURA_MOD_BLOCK_PERCENT: u32 = 51;
pub const AURA_MOD_CRIT_PERCENT: u32 = 52;
pub const AURA_PERIODIC_LEECH: u32 = 53;
pub const AURA_MOD_HIT_CHANCE: u32 = 54;
pub const AURA_MOD_SPELL_HIT_CHANCE: u32 = 55;
pub const AURA_TRANSFORM: u32 = 56;
pub const AURA_MOD_SPELL_CRIT_CHANCE: u32 = 57;
pub const AURA_MOD_INCREASE_SWIM_SPEED: u32 = 58;
pub const AURA_MOD_DAMAGE_DONE_CREATURE: u32 = 59;
pub const AURA_MOD_PACIFY_SILENCE: u32 = 60;
pub const AURA_MOD_SCALE: u32 = 61;
pub const AURA_PERIODIC_HEALTH_FUNNEL: u32 = 62;
pub const AURA_PERIODIC_MANA_LEECH: u32 = 64;
pub const AURA_MOD_CASTING_SPEED: u32 = 65;
pub const AURA_FEIGN_DEATH: u32 = 66;
pub const AURA_MOD_DISARM: u32 = 67;
pub const AURA_MOD_STALKED: u32 = 68;
pub const AURA_SCHOOL_ABSORB: u32 = 69;
pub const AURA_EXTRA_ATTACKS: u32 = 70;
pub const AURA_MOD_SPELL_CRIT_CHANCE_SCHOOL: u32 = 71;
pub const AURA_MOD_POWER_COST_SCHOOL_PCT: u32 = 72;
pub const AURA_MOD_POWER_COST_SCHOOL: u32 = 73;
pub const AURA_REFLECT_SPELLS_SCHOOL: u32 = 74;
pub const AURA_MOD_LANGUAGE: u32 = 75;
pub const AURA_FAR_SIGHT: u32 = 76;
pub const AURA_MECHANIC_IMMUNITY: u32 = 77;
pub const AURA_MOUNTED: u32 = 78;
pub const AURA_MOD_DAMAGE_PERCENT_DONE: u32 = 79;
pub const AURA_MOD_PERCENT_STAT: u32 = 80;
pub const AURA_SPLIT_DAMAGE_PCT: u32 = 81;
pub const AURA_WATER_BREATHING: u32 = 82;
pub const AURA_MOD_BASE_RESISTANCE: u32 = 83;
pub const AURA_MOD_REGEN: u32 = 84;
pub const AURA_MOD_POWER_REGEN: u32 = 85;
pub const AURA_CHANNEL_DEATH_ITEM: u32 = 86;
pub const AURA_MOD_DAMAGE_PERCENT_TAKEN: u32 = 87;
pub const AURA_MOD_HEALTH_REGEN_PERCENT: u32 = 88;
pub const AURA_PERIODIC_DAMAGE_PERCENT: u32 = 89;
pub const AURA_MOD_RESIST_CHANCE: u32 = 90;
pub const AURA_MOD_DETECT_RANGE: u32 = 91;
pub const AURA_PREVENTS_FLEEING: u32 = 92;
pub const AURA_MOD_UNATTACKABLE: u32 = 93;
pub const AURA_INTERRUPT_REGEN: u32 = 94;
pub const AURA_GHOST: u32 = 95;
pub const AURA_SPELL_MAGNET: u32 = 96;
pub const AURA_MANA_SHIELD: u32 = 97;
pub const AURA_MOD_SKILL_TALENT: u32 = 98;
pub const AURA_MOD_ATTACK_POWER: u32 = 99;
pub const AURA_AURAS_VISIBLE: u32 = 100;
pub const AURA_MOD_RESISTANCE_PCT: u32 = 101;
pub const AURA_MOD_MELEE_ATTACK_POWER_VERSUS: u32 = 102;
pub const AURA_MOD_TOTAL_THREAT: u32 = 103;
pub const AURA_WATER_WALK: u32 = 104;
pub const AURA_FEATHER_FALL: u32 = 105;
pub const AURA_HOVER: u32 = 106;
pub const AURA_ADD_FLAT_MODIFIER: u32 = 107;
pub const AURA_ADD_PCT_MODIFIER: u32 = 108;
pub const AURA_ADD_TARGET_TRIGGER: u32 = 109;
pub const AURA_MOD_POWER_REGEN_PERCENT: u32 = 110;
pub const AURA_ADD_CASTER_HIT_TRIGGER: u32 = 111;
pub const AURA_OVERRIDE_CLASS_SCRIPTS: u32 = 112;
pub const AURA_MOD_RANGED_DAMAGE_TAKEN: u32 = 113;
pub const AURA_MOD_RANGED_DAMAGE_TAKEN_PCT: u32 = 114;
pub const AURA_MOD_HEALING: u32 = 115;
pub const AURA_MOD_REGEN_DURING_COMBAT: u32 = 116;
pub const AURA_MOD_MECHANIC_RESISTANCE: u32 = 117;
pub const AURA_MOD_HEALING_PCT: u32 = 118;
pub const AURA_UNTRACKABLE: u32 = 120;
pub const AURA_EMPATHY: u32 = 121;
pub const AURA_MOD_OFFHAND_DAMAGE_PCT: u32 = 122;
pub const AURA_MOD_TARGET_RESISTANCE: u32 = 123;
pub const AURA_MOD_RANGED_ATTACK_POWER: u32 = 124;
pub const AURA_MOD_MELEE_DAMAGE_TAKEN: u32 = 125;
pub const AURA_MOD_MELEE_DAMAGE_TAKEN_PCT: u32 = 126;
pub const AURA_RANGED_ATTACK_POWER_ATTACKER_BONUS: u32 = 127;
pub const AURA_MOD_POSSESS_PET: u32 = 128;
pub const AURA_MOD_SPEED_ALWAYS: u32 = 129;
pub const AURA_MOD_MOUNTED_SPEED_ALWAYS: u32 = 130;
pub const AURA_MOD_RANGED_ATTACK_POWER_VERSUS: u32 = 131;
pub const AURA_MOD_INCREASE_ENERGY_PERCENT: u32 = 132;
pub const AURA_MOD_INCREASE_HEALTH_PERCENT: u32 = 133;
pub const AURA_MOD_MANA_REGEN_INTERRUPT: u32 = 134;
pub const AURA_MOD_HEALING_DONE: u32 = 135;
pub const AURA_MOD_HEALING_DONE_PERCENT: u32 = 136;
pub const AURA_MOD_TOTAL_STAT_PERCENTAGE: u32 = 137;
pub const AURA_MOD_HASTE: u32 = 138;
pub const AURA_FORCE_REACTION: u32 = 139;
pub const AURA_MOD_RANGED_HASTE: u32 = 140;
pub const AURA_MOD_RANGED_AMMO_HASTE: u32 = 141;
pub const AURA_MOD_BASE_RESISTANCE_PCT: u32 = 142;
pub const AURA_MOD_RESISTANCE_EXCLUSIVE: u32 = 143;
pub const AURA_SAFE_FALL: u32 = 144;
pub const AURA_CHARISMA: u32 = 145;
pub const AURA_PERSUADED: u32 = 146;
pub const AURA_MECHANIC_IMMUNITY_MASK: u32 = 147;
pub const AURA_RETAIN_COMBO_POINTS: u32 = 148;
pub const AURA_RESIST_PUSHBACK: u32 = 149;
pub const AURA_MOD_SHIELD_BLOCKVALUE_PCT: u32 = 150;
pub const AURA_TRACK_STEALTHED: u32 = 151;
pub const AURA_MOD_DETECTED_RANGE: u32 = 152;
pub const AURA_SPLIT_DAMAGE_FLAT: u32 = 153;
pub const AURA_MOD_STEALTH_LEVEL: u32 = 154;
pub const AURA_MOD_WATER_BREATHING: u32 = 155;
pub const AURA_MOD_REPUTATION_GAIN: u32 = 156;
pub const AURA_PET_DAMAGE_MULTI: u32 = 157;
pub const AURA_MOD_SHIELD_BLOCKVALUE: u32 = 158;
pub const AURA_NO_PVP_CREDIT: u32 = 159;
pub const AURA_MOD_AOE_AVOIDANCE: u32 = 160;
pub const AURA_MOD_HEALTH_REGEN_IN_COMBAT: u32 = 161;
pub const AURA_POWER_BURN_MANA: u32 = 162;
pub const AURA_MOD_CRIT_DAMAGE_BONUS_MELEE: u32 = 163;
pub const AURA_MELEE_ATTACK_POWER_ATTACKER_BONUS: u32 = 165;
pub const AURA_MOD_ATTACK_POWER_PCT: u32 = 166;
pub const AURA_MOD_RANGED_ATTACK_POWER_PCT: u32 = 167;
pub const AURA_MOD_DAMAGE_DONE_VERSUS: u32 = 168;
pub const AURA_MOD_CRIT_PERCENT_VERSUS: u32 = 169;
pub const AURA_DETECT_AMORE: u32 = 170;
pub const AURA_MOD_SPEED_NOT_STACK: u32 = 171;
pub const AURA_MOD_MOUNTED_SPEED_NOT_STACK: u32 = 172;
pub const AURA_ALLOW_CHAMPION_SPELLS: u32 = 173;
pub const AURA_MOD_SPELL_DAMAGE_OF_STAT_PERCENT: u32 = 174;
pub const AURA_MOD_SPELL_HEALING_OF_STAT_PERCENT: u32 = 175;

/// `SpellAuraInterruptFlags`, as Thistle Tea reads them.
pub const INTERRUPT_CAST: u32 = 0x1;
pub const INTERRUPT_DAMAGE: u32 = 0x2;
pub const INTERRUPT_ACTION: u32 = 0x4;
pub const INTERRUPT_MOVE: u32 = 0x8;
pub const INTERRUPT_TURNING: u32 = 0x10;
pub const INTERRUPT_ENTER_COMBAT: u32 = 0x20;
pub const INTERRUPT_NOT_MOUNTED: u32 = 0x40;
pub const INTERRUPT_UNDER_WATER: u32 = 0x80;
pub const INTERRUPT_ABOVE_WATER: u32 = 0x100;
pub const INTERRUPT_SHEATHING: u32 = 0x200;
pub const INTERRUPT_TALK: u32 = 0x400;
pub const INTERRUPT_USE: u32 = 0x800;
pub const INTERRUPT_ATTACK: u32 = 0x1000;
pub const INTERRUPT_SHAPESHIFT: u32 = 0x8000;
pub const INTERRUPT_ACTION_COMPLETE: u32 = 0x1_0000;
pub const INTERRUPT_MOUNTING: u32 = 0x2_0000;
pub const INTERRUPT_NOT_SEATED: u32 = 0x4_0000;
pub const INTERRUPT_CHANGE_MAP: u32 = 0x8_0000;
pub const INTERRUPT_TELEPORTED: u32 = 0x40_0000;
pub const INTERRUPT_DIRECT_DAMAGE: u32 = 0x100_0000;

// Slot flags.
const AFLAG_CANCELABLE: u32 = 0x1;
const AFLAG_EFF_INDEX: [u32; 3] = [0x8, 0x4, 0x2];

pub const MAX_SLOTS: usize = 48;
pub const POSITIVE_SLOTS: usize = 32;

/// Every aura type the server knows: those with an effect here or in the
/// stats, and those it keeps on the unit with no effect of their own.
pub fn supported(aura: u32) -> bool {
    aura <= 175 && !matches!(aura, 46 | 48 | 63 | 119 | 164)
}

/// One aura effect of a holder.
#[derive(Debug, Clone)]
pub struct Aura {
    pub effect: u8,
    pub kind: u32,
    /// The amount of one stack.
    pub amount: i32,
    pub misc: i32,
    pub trigger: u32,
    /// `EffectItemType`: the spells a modifier or trigger affects.
    pub mask: u32,
    pub period: Option<Duration>,
    pub next_tick: Option<Instant>,
    pub ticks: u32,
    /// What an absorb has left.
    pub left: i32,
}

/// What kind of aura only one of may be on a unit (for some, one per
/// caster).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusive {
    Shapeshift,
    HunterAspect,
    HunterSting,
    ShamanShield,
    Tracking,
    MagePolymorph,
    MageArmor,
    MageMagic,
    PositiveShout,
    PaladinSeal,
    PaladinBlessing,
    PaladinJudgement,
    PaladinAura,
    WarlockArmor,
    WarlockCurse,
    Food,
    Drink,
    FoodAndDrink,
    WellFed,
    BattleElixir,
    GuardianElixir,
    Flask,
    NegativeHaste,
    Snare,
}

impl Exclusive {
    /// Whether one of each caster's may be on a unit, rather than one in
    /// all.
    fn per_caster(self) -> bool {
        matches!(
            self,
            Exclusive::PaladinBlessing
                | Exclusive::WarlockCurse
                | Exclusive::HunterSting
                | Exclusive::PaladinJudgement
        )
    }

    fn conflicts(self, other: Exclusive) -> bool {
        use Exclusive::*;
        match (self, other) {
            (FoodAndDrink, Food | Drink | FoodAndDrink) | (Food | Drink, FoodAndDrink) => true,
            (Flask, BattleElixir | GuardianElixir) | (BattleElixir | GuardianElixir, Flask) => true,
            (a, b) => a == b,
        }
    }
}

const FAMILY_MAGE: u32 = 3;
const FAMILY_WARRIOR: u32 = 4;
const FAMILY_WARLOCK: u32 = 5;
const FAMILY_HUNTER: u32 = 9;
const FAMILY_PALADIN: u32 = 10;
const FAMILY_SHAMAN: u32 = 11;
const DISPEL_POISON: u32 = 4;
const DISPEL_CURSE: u32 = 2;

/// The exclusive kind of a spell, as Thistle Tea's `exclusive_category`
/// works it out.
pub fn exclusive(s: &SpellTemplate, elixir: Option<u32>) -> Option<Exclusive> {
    let flags = s.family_flags as u32;
    let flags_hi = (s.family_flags >> 32) as u32;
    let aura = |a: u32| s.effects.iter().any(|e| e.effect == 6 && e.aura == a);
    // Food, drink and the well fed.
    if s.family_name == 0 {
        if s.aura_interrupt_flags & INTERRUPT_NOT_SEATED != 0 {
            let food = s.effects.iter().any(|e| matches!(e.aura, 20 | 84));
            let drink = s.effects.iter().any(|e| matches!(e.aura, 21 | 85));
            match (food, drink) {
                (true, true) => return Some(Exclusive::FoodAndDrink),
                (true, false) => return Some(Exclusive::Food),
                (false, true) => return Some(Exclusive::Drink),
                _ => {}
            }
        } else if s.attributes[2] & 0x8000_0000 != 0 {
            return Some(Exclusive::WellFed);
        }
    }
    if s.family_name == 6
        && matches!(s.spell_icon, 52 | 79)
        && s.attr(0, sd::ATTR_ALLOW_WHILE_SITTING)
        && s.interrupt_flags & 0x8 != 0
    {
        return Some(Exclusive::WellFed);
    }
    if aura(AURA_MOD_SHAPESHIFT) {
        return Some(Exclusive::Shapeshift);
    }
    if s.id == 13161
        || (s.family_name == FAMILY_HUNTER && s.spell_icon != 0 && active_icon(s) == 122 && s.id != 75)
    {
        return Some(Exclusive::HunterAspect);
    }
    if s.family_name == FAMILY_HUNTER && s.dispel == DISPEL_POISON {
        return Some(Exclusive::HunterSting);
    }
    if s.id == 23552 || (s.family_name == FAMILY_SHAMAN && flags & 0x400 != 0) {
        return Some(Exclusive::ShamanShield);
    }
    let tracking = s
        .effects
        .iter()
        .any(|e| e.effect == 6 && matches!(e.aura, 44 | 45 | 151));
    if tracking && (s.attr(0, sd::ATTR_ALLOW_WHILE_MOUNTED) || s.attributes[1] & 0x2_0000 != 0) {
        return Some(Exclusive::Tracking);
    }
    if s.family_name == FAMILY_MAGE
        && s.effects[0].aura == AURA_MOD_CONFUSE
        && s.prevention_type == 1
    {
        return Some(Exclusive::MagePolymorph);
    }
    if s.family_name == FAMILY_WARRIOR && (flags & 0x0001_0000 != 0 || flags_hi & 0x8000 != 0) {
        // The helpful shouts (Battle Shout), by Thistle Tea's family masks.
        return Some(Exclusive::PositiveShout);
    }
    if s.family_name == FAMILY_MAGE {
        if flags & 0x1200_0000 != 0 {
            return Some(Exclusive::MageArmor);
        }
        if flags & 0x2000 != 0 {
            return Some(Exclusive::MageMagic);
        }
    }
    if s.family_name == FAMILY_PALADIN {
        if flags & 0x0A00_0200 != 0 {
            return Some(Exclusive::PaladinSeal);
        }
        if flags & 0x1000_0100 != 0 {
            return Some(Exclusive::PaladinBlessing);
        }
        if (flags & 0x2018_0400 != 0 && s.base_level != 0)
            || (s.spell_icon == 561 && s.spell_visual == 5652)
        {
            return Some(Exclusive::PaladinJudgement);
        }
        if s.effects.iter().any(|e| e.effect == 35) {
            return Some(Exclusive::PaladinAura);
        }
    }
    if s.family_name == FAMILY_WARLOCK {
        if s.spell_visual == 130 && s.spell_icon == 89 {
            return Some(Exclusive::WarlockArmor);
        }
        if s.dispel == DISPEL_CURSE {
            return Some(Exclusive::WarlockCurse);
        }
    }
    if let Some(mask) = elixir {
        if mask & 0x3 == 0x3 {
            return Some(Exclusive::Flask);
        }
        if mask & 0x10 != 0 {
            return Some(Exclusive::WellFed);
        }
        if mask & 0x1 != 0 {
            return Some(Exclusive::BattleElixir);
        }
        if mask & 0x2 != 0 {
            return Some(Exclusive::GuardianElixir);
        }
    }
    if s.family_name != 13 {
        let only = |a: u32| {
            aura(a)
                && s
                    .effects
                    .iter()
                    .all(|e| e.effect != 6 || e.aura == a)
        };
        if only(AURA_MOD_HASTE)
            && s
                .effects
                .iter()
                .any(|e| e.aura == AURA_MOD_HASTE && e.base_points + e.base_dice < 0)
        {
            return Some(Exclusive::NegativeHaste);
        }
        // A snare, but not a daze.
        if only(AURA_MOD_DECREASE_SPEED) && s.attributes[1] & 0x0040_0000 == 0 {
            return Some(Exclusive::Snare);
        }
    }
    None
}

/// The active icon is not loaded; aspects all use 122 as their icon's
/// active one and their own icons differ, so the aspects are told by name.
fn active_icon(s: &SpellTemplate) -> u32 {
    if s.name.starts_with("Aspect of") {
        122
    } else {
        0
    }
}

/// A spell's aura effects on a unit from one caster.
#[derive(Debug, Clone)]
pub struct Holder {
    pub spell: u32,
    pub caster: Guid,
    /// The item whose spell it is: an equip spell, an enchantment, or the
    /// item a cast used.
    pub item: Option<Guid>,
    /// From the gear the unit wears: it goes when the item comes off.
    pub equipped: bool,
    pub slot: Option<usize>,
    pub positive: bool,
    pub passive: bool,
    pub stacks: u32,
    /// Charges left; 0 for none.
    pub charges: u32,
    pub expires: Option<Instant>,
    /// The full duration, in milliseconds.
    pub duration: Option<u32>,
    pub level: u32,
    pub school: u32,
    pub mechanic: u32,
    pub dispel: u32,
    pub interrupt_flags: u32,
    /// The weapon the spell wants: class, subclass mask and inventory types.
    pub item_class: i32,
    pub item_subclass_mask: i32,
    pub item_inventory_mask: i32,
    pub exclusive: Option<Exclusive>,
    pub combo: u8,
    /// A proc's own cooldown.
    pub proc_ready_at: Option<Instant>,
    pub effects: Vec<Aura>,
}

impl Holder {
    pub fn has(&self, kind: u32) -> bool {
        self.effects.iter().any(|a| a.kind == kind)
    }

    /// Whether the holder applies whatever the unit wields.
    pub fn unrestricted(&self) -> bool {
        self.item_class <= 0 && self.item_inventory_mask <= 0
    }
}

/// A unit's auras.
#[derive(Debug, Clone, Default)]
pub struct Auras {
    pub holders: Vec<Holder>,
}

impl Auras {
    /// Every aura effect with its holder.
    pub fn effects(&self) -> impl Iterator<Item = (&Holder, &Aura)> {
        self.holders
            .iter()
            .flat_map(|h| h.effects.iter().map(move |a| (h, a)))
    }

    fn of(&self, kind: u32) -> impl Iterator<Item = (&Holder, &Aura)> {
        self.effects().filter(move |(_, a)| a.kind == kind)
    }

    pub fn has(&self, kind: u32) -> bool {
        self.of(kind).next().is_some()
    }

    pub fn has_spell(&self, spell: u32) -> bool {
        self.holders.iter().any(|h| h.spell == spell)
    }

    pub fn is_empty(&self) -> bool {
        self.holders.is_empty()
    }

    /// The sum of every `kind` aura's amount, with stacks.
    pub fn sum(&self, kind: u32) -> i32 {
        self.of(kind)
            .map(|(h, a)| a.amount * h.stacks.max(1) as i32)
            .sum()
    }

    /// The sum of `kind` auras whose misc value is `misc`, or -1 (all).
    pub fn sum_misc(&self, kind: u32, misc: i32) -> i32 {
        self.of(kind)
            .filter(|(_, a)| a.misc == misc || a.misc == -1)
            .map(|(h, a)| a.amount * h.stacks.max(1) as i32)
            .sum()
    }

    /// The sum of `kind` auras whose misc value is a mask covering `bit`.
    pub fn sum_mask(&self, kind: u32, bit: u32) -> i32 {
        self.of(kind)
            .filter(|(_, a)| (a.misc as u32) & bit != 0)
            .map(|(h, a)| a.amount * h.stacks.max(1) as i32)
            .sum()
    }

    /// The product of `kind` auras as percentages: +10 and +20 give 1.32.
    pub fn product(&self, kind: u32) -> f32 {
        self.of(kind).fold(1.0, |m, (h, a)| {
            m * ((100 + a.amount * h.stacks.max(1) as i32).max(0) as f32 / 100.0)
        })
    }

    pub fn product_misc(&self, kind: u32, misc: i32) -> f32 {
        self.of(kind)
            .filter(|(_, a)| a.misc == misc || a.misc == -1)
            .fold(1.0, |m, (h, a)| {
                m * ((100 + a.amount * h.stacks.max(1) as i32).max(0) as f32 / 100.0)
            })
    }

    pub fn product_mask(&self, kind: u32, bit: u32) -> f32 {
        self.of(kind)
            .filter(|(_, a)| (a.misc as u32) & bit != 0)
            .fold(1.0, |m, (h, a)| {
                m * ((100 + a.amount * h.stacks.max(1) as i32).max(0) as f32 / 100.0)
            })
    }

    pub fn max(&self, kind: u32) -> i32 {
        self.of(kind)
            .map(|(h, a)| a.amount * h.stacks.max(1) as i32)
            .max()
            .unwrap_or(0)
    }

    pub fn min(&self, kind: u32) -> i32 {
        self.of(kind)
            .map(|(h, a)| a.amount * h.stacks.max(1) as i32)
            .min()
            .unwrap_or(0)
    }

    /// The speed multiplier the auras give, as a percentage change: the
    /// best haste, and the worst slow.
    pub fn speed_percent(&self, mounted: bool) -> i32 {
        let mut up = self
            .max(AURA_MOD_INCREASE_SPEED)
            .max(self.max(AURA_MOD_SPEED_ALWAYS))
            .max(self.max(AURA_MOD_SPEED_NOT_STACK));
        if mounted {
            up += self
                .max(AURA_MOD_INCREASE_MOUNTED_SPEED)
                .max(self.max(AURA_MOD_MOUNTED_SPEED_ALWAYS))
                .max(self.max(AURA_MOD_MOUNTED_SPEED_NOT_STACK));
        }
        let down = self.min(AURA_MOD_DECREASE_SPEED);
        up + down
    }

    /// Whether `spell`, a school `mask`, is stopped by an immunity.
    pub fn school_immune(&self, mask: u32) -> bool {
        self.of(AURA_SCHOOL_IMMUNITY)
            .chain(self.of(AURA_DAMAGE_IMMUNITY))
            .any(|(_, a)| (a.misc as u32) & mask != 0 && mask != 0)
    }

    /// Whether a mechanic is stopped.
    pub fn mechanic_immune(&self, mechanic: u32) -> bool {
        mechanic != 0
            && (self
                .of(AURA_MECHANIC_IMMUNITY)
                .any(|(_, a)| a.misc as u32 == mechanic)
                || self
                    .of(AURA_MECHANIC_IMMUNITY_MASK)
                    .any(|(_, a)| (a.misc as u32) & (1 << (mechanic - 1).min(31)) != 0))
    }

    pub fn dispel_immune(&self, dispel: u32) -> bool {
        dispel != 0
            && self
                .of(AURA_DISPEL_IMMUNITY)
                .any(|(_, a)| a.misc as u32 == dispel)
    }

    fn slot_taken(&self, slot: usize) -> bool {
        self.holders.iter().any(|h| h.slot == Some(slot))
    }
}

/// Whether a spell is helpful: no harmful effects, aimed at a friend.
pub fn positive(s: &SpellTemplate) -> bool {
    if s.attr(0, sd::ATTR_DEBUFF) {
        return false;
    }
    !s.effects.iter().any(|e| {
        matches!(e.effect, 2 | 9 | 17 | 31 | 58 | 62 | 121)
            || matches!(
                e.aura,
                AURA_PERIODIC_DAMAGE
                    | AURA_MOD_STUN
                    | AURA_MOD_ROOT
                    | AURA_MOD_SILENCE
                    | AURA_MOD_DECREASE_SPEED
                    | AURA_MOD_FEAR
                    | AURA_MOD_CONFUSE
                    | AURA_MOD_PACIFY
                    | AURA_MOD_PACIFY_SILENCE
                    | AURA_PERIODIC_LEECH
                    | AURA_PERIODIC_MANA_LEECH
                    | AURA_PERIODIC_DAMAGE_PERCENT
                    | AURA_POWER_BURN_MANA
                    | AURA_MOD_CHARM
                    | AURA_MOD_POSSESS
                    | AURA_MOD_TAUNT
                    | AURA_MOD_STALKED
                    | AURA_MOD_DISARM
                    | AURA_TRANSFORM
                    | AURA_MOD_DETECT_RANGE
            )
            || (matches!(
                e.aura,
                AURA_MOD_STAT
                    | AURA_MOD_RESISTANCE
                    | AURA_MOD_DAMAGE_DONE
                    | AURA_MOD_DAMAGE_PERCENT_DONE
                    | AURA_MOD_ATTACK_POWER
                    | AURA_MOD_RANGED_ATTACK_POWER
                    | AURA_MOD_HASTE
                    | AURA_MOD_RANGED_HASTE
                    | AURA_MOD_ATTACKSPEED
                    | AURA_MOD_INCREASE_HEALTH
                    | AURA_MOD_HIT_CHANCE
                    | AURA_MOD_SPELL_HIT_CHANCE
                    | AURA_MOD_HEALING_PCT
                    | AURA_MOD_RESISTANCE_PCT
                    | AURA_MOD_BASE_RESISTANCE_PCT
                    | AURA_MOD_CASTING_SPEED
                    | AURA_MOD_PERCENT_STAT
                    | AURA_MOD_TOTAL_STAT_PERCENTAGE
            ) && e.base_points + e.base_dice < 0)
            || (matches!(
                e.aura,
                AURA_MOD_DAMAGE_TAKEN | AURA_MOD_DAMAGE_PERCENT_TAKEN | AURA_MOD_MELEE_DAMAGE_TAKEN
            ) && e.base_points + e.base_dice > 0)
            || matches!(e.target_a, 6 | 15 | 16 | 22 | 24 | 28) && e.effect != 0
    })
}

/// The auras of `g`, mutably.
pub fn auras_mut(map: &mut Map, g: Guid) -> Option<&mut Auras> {
    if map.players.contains_key(&g) {
        return map.players.get_mut(&g).map(|p| &mut p.auras);
    }
    map.creature_mut(g).map(|c| &mut c.auras)
}

pub fn auras(map: &Map, g: Guid) -> Option<&Auras> {
    if let Some(p) = map.players.get(&g) {
        return Some(&p.auras);
    }
    map.creature(g).map(|c| &c.auras)
}

/// How an aura is applied: what item it is from, and what the cast knew.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    pub item: Option<Guid>,
    pub equipped: bool,
    pub combo: u8,
    /// A duration the cast worked out (combo points, modifiers); `None` to
    /// take the spell's.
    pub duration: Option<Option<u32>>,
    /// Charges the cast worked out.
    pub charges: Option<u32>,
}

/// Why an aura couldn't be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// A stronger rank or a higher spell of its group is there.
    Bounced,
    Immune,
}

/// Put `spell`'s effect `index` on `target`.
pub fn apply(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    spell: &SpellTemplate,
    index: usize,
    amount: i32,
) {
    let _ = apply_spell(
        map,
        caster,
        target,
        spell,
        &[(index, amount)],
        Context::default(),
    );
}

/// Put `spell`'s aura effects (`effects`: each index and its amount) on
/// `target` as one holder.
pub fn apply_spell(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    spell: &SpellTemplate,
    effects: &[(usize, i32)],
    ctx: Context,
) -> Result<(), Refused> {
    let now = map.now;
    let data = map.realm.data.clone();
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    let duration = ctx
        .duration
        .unwrap_or_else(|| data.spells.duration(spell, level));
    let pos = positive(spell);
    let passive = spell.passive();
    let Some(existing) = auras(map, target) else {
        return Ok(());
    };
    // Immunities: a harmful spell's mechanic or school.
    if !pos && !spell.attr(0, sd::ATTR_NO_IMMUNITIES) {
        let mechanic_immune = existing.mechanic_immune(spell.mechanic)
            || effects
                .iter()
                .any(|(i, _)| existing.mechanic_immune(spell.effects[*i].mechanic));
        if mechanic_immune
            || (!spell.attr(2, sd::ATTR_EX2_NO_SCHOOL_IMMUNITIES)
                && existing.school_immune(spell.school_mask()))
            || existing.dispel_immune(spell.dispel)
        {
            return Err(Refused::Immune);
        }
    }
    let excl = exclusive(spell, data.spells.elixirs.get(&spell.id).copied());
    // A stronger rank of a helpful spell already there blocks a weaker.
    if pos && !passive {
        let blocked = existing.holders.iter().any(|h| {
            h.spell != spell.id
                && data.spells.same_kind(h.spell, spell.id)
                && data
                    .spells
                    .ranks
                    .get(&h.spell)
                    .zip(data.spells.ranks.get(&spell.id))
                    .is_some_and(|(a, b)| a.rank > b.rank)
                && (h.caster == caster || !cross_caster_coexist(spell, h, effects))
        });
        let group_blocked = group_relation(&data, existing, spell) == Some(GroupRelation::Blocked);
        if blocked || group_blocked {
            return Err(Refused::Bounced);
        }
    }
    let mut aura_effects = Vec::new();
    for &(index, amount) in effects {
        let e = spell.effects[index];
        let period = (e.amplitude > 0).then(|| Duration::from_millis(u64::from(e.amplitude)));
        aura_effects.push(Aura {
            effect: index as u8,
            kind: e.aura,
            amount,
            misc: e.misc_value,
            trigger: e.trigger_spell,
            mask: e.item_type,
            period,
            next_tick: period.map(|p| now + p),
            ticks: 0,
            left: amount,
        });
    }
    let holder = Holder {
        spell: spell.id,
        caster,
        item: ctx.item,
        equipped: ctx.equipped,
        slot: None,
        positive: pos,
        passive,
        stacks: 1,
        charges: ctx.charges.unwrap_or(spell.proc_charges),
        expires: duration.map(|ms| now + Duration::from_millis(u64::from(ms))),
        duration,
        level,
        school: spell.school_mask(),
        mechanic: spell.mechanic,
        dispel: spell.dispel,
        interrupt_flags: spell.aura_interrupt_flags,
        item_class: spell.equipped_item_class,
        item_subclass_mask: spell.equipped_item_subclass_mask,
        item_inventory_mask: spell.equipped_item_inventory_type_mask,
        exclusive: excl,
        combo: ctx.combo,
        proc_ready_at: None,
        effects: aura_effects,
    };
    let visible = visible(spell, caster == target);
    // What the new holder replaces.
    let replaced: Vec<usize> = existing
        .holders
        .iter()
        .enumerate()
        .filter(|(_, h)| replaces(&data, spell, &holder, h, effects))
        .map(|(i, _)| i)
        .collect();
    let mut gone = Vec::new();
    let mut kept_slot = None;
    let mut refreshed = None;
    {
        let Some(a) = auras_mut(map, target) else {
            return Ok(());
        };
        // The same spell from the same caster and item: refreshed, and a
        // stack more.
        if let Some(i) = a
            .holders
            .iter()
            .position(|h| h.spell == spell.id && h.caster == caster && h.item == ctx.item)
        {
            let old = &mut a.holders[i];
            let stacks = if spell.stack_amount > 1 {
                (old.stacks + 1).min(spell.stack_amount)
            } else {
                1
            };
            let ticks: Vec<(u8, Option<Instant>, u32)> = old
                .effects
                .iter()
                .map(|x| (x.effect, x.next_tick, x.ticks))
                .collect();
            let slot = old.slot;
            *old = Holder {
                stacks,
                slot,
                ..holder.clone()
            };
            // A stacking aura keeps its ticks' rhythm.
            if spell.stack_amount > 1 {
                for x in old.effects.iter_mut() {
                    if let Some((_, next, n)) = ticks.iter().find(|t| t.0 == x.effect) {
                        x.next_tick = *next;
                        x.ticks = *n;
                    }
                }
            }
            refreshed = Some(i);
        }
        let mut removed = 0;
        for i in replaced {
            let i = i - removed;
            if refreshed == Some(i + removed) {
                continue;
            }
            let h = a.holders.remove(i);
            if refreshed.is_some_and(|r| r > i + removed) {
                refreshed = refreshed.map(|r| r - 1);
            }
            removed += 1;
            if h.slot.is_some() && kept_slot.is_none() && visible {
                kept_slot = h.slot;
            }
            gone.push(h);
        }
        if refreshed.is_none() {
            a.holders.push(holder);
        }
    }
    for h in &gone {
        if let Some(slot) = h.slot {
            clear_slot(map, target, slot);
        }
    }
    for h in gone {
        for x in &h.effects {
            ended(map, target, &h, x);
        }
    }
    // A slot for the new holder.
    let index = refreshed.unwrap_or_else(|| auras(map, target).map_or(0, |a| a.holders.len() - 1));
    if visible {
        let slot = {
            let fields_taken: Vec<bool> = map
                .unit_fields(target)
                .map(|f| {
                    (0..MAX_SLOTS)
                        .map(|i| f.get_u32(UNIT_FIELD_AURA + i as u16) != 0)
                        .collect()
                })
                .unwrap_or_else(|| vec![false; MAX_SLOTS]);
            let a = auras(map, target).expect("the unit is here");
            a.holders[index].slot.or(kept_slot).or_else(|| {
                let range = if pos { 0..POSITIVE_SLOTS } else { POSITIVE_SLOTS..MAX_SLOTS };
                range.into_iter().find(|i| !a.slot_taken(*i) && !fields_taken[*i])
            })
        };
        if let Some(a) = auras_mut(map, target) {
            a.holders[index].slot = slot;
        }
        if let Some(slot) = slot {
            write_slot(map, target, index);
            if let (Some(p), Some(ms)) = (map.players.get(&target), duration) {
                p.outbound.send(&SMSG_UPDATE_AURA_DURATION {
                    aura_slot: slot as u8,
                    aura_duration: ms,
                });
            }
        }
    }
    let h = auras(map, target).map(|a| a.holders[index].clone());
    if let Some(h) = h
        && refreshed.is_none()
    {
        for x in &h.effects {
            started(map, target, &h, x);
        }
    }
    recalc(map, target);
    Ok(())
}

/// Whether an aura shows on the unit's bar.
pub fn visible(s: &SpellTemplate, on_self: bool) -> bool {
    if s.attributes[0] == sd::ATTR_HIDDEN && s.duration_index == 21 {
        return false;
    }
    let area = s.effects.iter().any(|e| e.effect == 35);
    if area {
        return !on_self || !s.passive();
    }
    !s.passive() || s.spell_visual > 0
}

/// Effects that let a spell's auras from two casters stand together:
/// periodic damage and healing, and dummies.
fn cross_caster_coexist(s: &SpellTemplate, h: &Holder, effects: &[(usize, i32)]) -> bool {
    const PERSONAL: &[u32] = &[
        AURA_DUMMY,
        AURA_PERIODIC_DAMAGE_PERCENT,
        AURA_PERIODIC_DAMAGE,
        AURA_PERIODIC_LEECH,
        AURA_PERIODIC_HEALTH_FUNNEL,
        AURA_PERIODIC_HEAL,
        AURA_PERIODIC_MANA_LEECH,
        AURA_CHANNEL_DEATH_ITEM,
    ];
    if s.channeled() || s.attr(3, sd::ATTR_EX3_DOT_STACKING) {
        return true;
    }
    effects.iter().all(|(i, _)| {
        !h.effects.iter().any(|x| x.effect as usize == *i)
            || PERSONAL.contains(&s.effects[*i].aura)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupRelation {
    Replaces,
    Blocked,
}

/// How `spell` stands to the spells of its groups already on the unit:
/// rule 1 replaces, rule 3 keeps the higher of the group's order.
fn group_relation(
    data: &crate::game::data::GameData,
    a: &Auras,
    spell: &SpellTemplate,
) -> Option<GroupRelation> {
    let mine = data.spells.groups.get(&spell.id)?;
    for h in &a.holders {
        if h.spell == spell.id || h.passive || data.spells.same_kind(h.spell, spell.id) {
            continue;
        }
        let Some(theirs) = data.spells.groups.get(&h.spell) else {
            continue;
        };
        for g in mine.iter().filter(|g| theirs.contains(g)) {
            match data.spells.group_rules.get(g).copied().unwrap_or(0) {
                1 => return Some(GroupRelation::Replaces),
                3 => {
                    return Some(if h.spell > spell.id {
                        GroupRelation::Blocked
                    } else {
                        GroupRelation::Replaces
                    });
                }
                _ => {}
            }
        }
    }
    None
}

/// Whether the new holder `new` (of `spell`) takes `old`'s place.
fn replaces(
    data: &crate::game::data::GameData,
    spell: &SpellTemplate,
    new: &Holder,
    old: &Holder,
    effects: &[(usize, i32)],
) -> bool {
    if old.passive && old.item.is_some() {
        return false;
    }
    if old.spell == new.spell && old.caster == new.caster {
        // Refreshed in place, unless from another item.
        return old.item != new.item && old.item.is_some() && new.item.is_some() && false;
    }
    // Control: one charm or possession.
    let control = |h: &Holder| h.has(AURA_MOD_CHARM) || h.has(AURA_MOD_POSSESS);
    if control(old) && control(new) {
        return true;
    }
    if let (Some(a), Some(b)) = (old.exclusive, new.exclusive)
        && old.spell != new.spell
        && a.conflicts(b)
        && (!a.per_caster() || old.caster == new.caster)
    {
        return true;
    }
    if new.has(AURA_MOD_SHAPESHIFT) && old.has(AURA_MOD_SHAPESHIFT) {
        return true;
    }
    if new.has(AURA_MOUNTED) && old.has(AURA_MOUNTED) {
        return true;
    }
    if old.spell == new.spell {
        // Another caster's: replaced unless the effects stand together.
        return !cross_caster_coexist(spell, old, effects);
    }
    if data.spells.same_kind(old.spell, new.spell) {
        return old.caster == new.caster || !cross_caster_coexist(spell, old, effects);
    }
    if !old.passive {
        let mine = data.spells.groups.get(&new.spell);
        let theirs = data.spells.groups.get(&old.spell);
        if let (Some(mine), Some(theirs)) = (mine, theirs) {
            for g in mine.iter().filter(|g| theirs.contains(g)) {
                match data.spells.group_rules.get(g).copied().unwrap_or(0) {
                    1 => return true,
                    3 => return old.spell <= new.spell,
                    _ => {}
                }
            }
        }
    }
    false
}

/// The slot fields for holder `index` of `target`.
fn write_slot(map: &mut Map, target: Guid, index: usize) {
    let Some(h) = auras(map, target).and_then(|a| a.holders.get(index)).cloned() else {
        return;
    };
    let Some(slot) = h.slot else {
        return;
    };
    let mut flags = if h.positive { AFLAG_CANCELABLE } else { 0 };
    for x in &h.effects {
        flags |= AFLAG_EFF_INDEX[(x.effect as usize).min(2)];
    }
    let apps = if h.charges > 0 {
        (h.charges * h.stacks.max(1)).min(255) - 1
    } else {
        h.stacks.clamp(1, 255) - 1
    };
    set_slot_fields(map, target, slot, h.spell, flags, h.level, apps);
}

fn clear_slot(map: &mut Map, target: Guid, slot: usize) {
    set_slot_fields(map, target, slot, 0, 0, 0, 0);
}

fn set_slot_fields(
    map: &mut Map,
    target: Guid,
    slot: usize,
    spell: u32,
    flags: u32,
    level: u32,
    apps: u32,
) {
    let Some(f) = map.unit_fields_mut(target) else {
        return;
    };
    let slot = slot as u16;
    f.set_u32(UNIT_FIELD_AURA + slot, spell);
    let flags_word = UNIT_FIELD_AURAFLAGS + slot / 8;
    let shift = (slot % 8) * 4;
    let old = f.get_u32(flags_word);
    f.set_u32(flags_word, (old & !(0xF << shift)) | ((flags & 0xF) << shift));
    let byte = (slot % 4) as u8;
    f.set_byte(
        UNIT_FIELD_AURALEVELS + slot / 4,
        byte,
        if spell == 0 { 0 } else { level.min(255) as u8 },
    );
    f.set_byte(
        UNIT_FIELD_AURAAPPLICATIONS + slot / 4,
        byte,
        if spell == 0 { 0 } else { apps as u8 },
    );
}

/// Take the holders `pick` chooses off `target`.
pub fn remove_where(map: &mut Map, target: Guid, mut pick: impl FnMut(&Holder) -> bool) {
    let Some(a) = auras_mut(map, target) else {
        return;
    };
    let mut gone = Vec::new();
    let mut i = 0;
    while i < a.holders.len() {
        if pick(&a.holders[i]) {
            gone.push(a.holders.remove(i));
        } else {
            i += 1;
        }
    }
    if gone.is_empty() {
        return;
    }
    for h in &gone {
        if let Some(slot) = h.slot {
            clear_slot(map, target, slot);
        }
    }
    for h in &gone {
        for x in &h.effects {
            ended(map, target, h, x);
        }
        removed(map, target, h);
    }
    recalc(map, target);
}

/// Take every aura of `spell` from `caster` off `target`.
pub fn remove_spell(map: &mut Map, target: Guid, spell: u32, caster: Option<Guid>) {
    remove_where(map, target, |h| {
        h.spell == spell && caster.is_none_or(|c| h.caster == c)
    });
}

/// Take the auras an item gave off its wearer.
pub fn remove_item(map: &mut Map, target: Guid, item: Guid) {
    remove_where(map, target, |h| h.item == Some(item));
}

/// Take off auras the unit's action breaks (`flags`: the interrupt flags).
pub fn interrupt(map: &mut Map, target: Guid, flags: u32) {
    let any = auras(map, target)
        .is_some_and(|a| a.holders.iter().any(|h| h.interrupt_flags & flags != 0));
    if any {
        remove_where(map, target, |h| h.interrupt_flags & flags != 0);
    }
}

/// One stack (or, with no stacks, the whole holder) of `spell` comes off.
pub fn remove_stack(map: &mut Map, target: Guid, spell: u32) {
    let Some(a) = auras_mut(map, target) else {
        return;
    };
    let Some(i) = a.holders.iter().position(|h| h.spell == spell) else {
        return;
    };
    if a.holders[i].stacks > 1 {
        a.holders[i].stacks -= 1;
        write_slot(map, target, i);
        recalc(map, target);
    } else {
        remove_where(map, target, |h| h.spell == spell);
    }
}

/// A charge of holder `spell` (from `caster`) is spent; with none left it
/// goes.
pub fn spend_charge(map: &mut Map, target: Guid, spell: u32, caster: Guid) {
    let Some(a) = auras_mut(map, target) else {
        return;
    };
    let Some(i) = a
        .holders
        .iter()
        .position(|h| h.spell == spell && h.caster == caster)
    else {
        return;
    };
    if a.holders[i].charges > 1 {
        a.holders[i].charges -= 1;
        write_slot(map, target, i);
    } else if a.holders[i].charges == 1 {
        remove_where(map, target, |h| h.spell == spell && h.caster == caster);
    }
}

/// An aura effect began: what it does at once.
fn started(map: &mut Map, target: Guid, h: &Holder, a: &Aura) {
    let kind = a.kind;
    let misc = a.misc;
    match kind {
        AURA_MOD_STUN | AURA_MOD_ROOT => {
            crate::game::map::combat::stop_attack(map, target);
            if kind == AURA_MOD_STUN
                && let Some(f) = map.unit_fields_mut(target)
            {
                f.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_STUNNED,
                    true,
                );
            }
            if let Some(p) = map.players.get(&target) {
                p.outbound.send(&SMSG_FORCE_MOVE_ROOT {
                    guid: target,
                    counter: 0,
                });
            }
            if kind == AURA_MOD_STUN {
                crate::game::map::spells::interrupt(map, target);
            }
        }
        AURA_MOD_FEAR | AURA_MOD_CONFUSE => {
            crate::game::map::combat::stop_attack(map, target);
            crate::game::map::spells::interrupt(map, target);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(
                    UNIT_FIELD_FLAGS,
                    if kind == AURA_MOD_FEAR {
                        UNIT_FLAG_FLEEING
                    } else {
                        UNIT_FLAG_CONFUSED
                    },
                    true,
                );
            }
        }
        AURA_MOD_SILENCE | AURA_MOD_PACIFY_SILENCE => {
            crate::game::map::spells::interrupt(map, target);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SILENCED, true);
            }
        }
        AURA_MOD_PACIFY => {
            crate::game::map::combat::stop_attack(map, target);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PACIFIED, true);
            }
        }
        AURA_MOD_SHAPESHIFT => {
            shapeshift(map, target, misc.max(0) as u8, h.spell);
        }
        AURA_TRANSFORM => {
            let display = map
                .realm
                .data
                .creature_template(misc.max(0) as u32)
                .and_then(|t| t.display_ids.iter().copied().find(|d| *d != 0))
                .unwrap_or(0);
            if display != 0
                && let Some(f) = map.unit_fields_mut(target)
            {
                f.set_u32(UNIT_FIELD_DISPLAYID, display);
            }
        }
        AURA_MOUNTED => {
            let display = map
                .realm
                .data
                .creature_template(misc as u32)
                .and_then(|t| t.display_ids.iter().copied().find(|d| *d != 0))
                .unwrap_or(0);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(UNIT_FIELD_MOUNTDISPLAYID, display);
            }
        }
        AURA_TRACK_CREATURES => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(PLAYER_TRACK_CREATURES, 1 << (misc - 1).clamp(0, 31));
            }
        }
        AURA_TRACK_RESOURCES => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(PLAYER_TRACK_RESOURCES, 1 << (misc - 1).clamp(0, 31));
            }
        }
        AURA_MOD_SCALE => {
            if let Some(f) = map.unit_fields_mut(target) {
                let s = f.get_f32(OBJECT_FIELD_SCALE_X);
                f.set_f32(OBJECT_FIELD_SCALE_X, s * (100 + a.amount) as f32 / 100.0);
            }
        }
        AURA_MOD_DISARM => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_DISARMED, true);
            }
        }
        AURA_FEIGN_DEATH => {
            crate::game::map::combat::leave_combat(map, target);
            crate::game::map::ai::forget(map, target);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FEIGN_DEATH, true);
                f.set_flag(UNIT_DYNAMIC_FLAGS, 0x20, true);
            }
        }
        AURA_MOD_TAUNT => {
            crate::game::map::ai::taunted(map, target, h.caster);
        }
        AURA_MOD_STEALTH | AURA_MOD_INVISIBILITY => {
            crate::game::map::combat::stop_attack(map, target);
            if kind == AURA_MOD_STEALTH
                && let Some(f) = map.unit_fields_mut(target)
            {
                f.set_byte(UNIT_FIELD_BYTES_1, 2, 0x02 | f.get_byte(UNIT_FIELD_BYTES_1, 2));
            }
            crate::game::map::visibility::stealth_changed(map, target);
        }
        AURA_WATER_WALK | AURA_FEATHER_FALL | AURA_HOVER => {
            if let Some(p) = map.players.get(&target) {
                match kind {
                    AURA_WATER_WALK => {
                        p.outbound.send(&SMSG_MOVE_WATER_WALK {
                            guid: target,
                            counter: 0,
                        });
                    }
                    AURA_FEATHER_FALL => {
                        p.outbound.send(&SMSG_MOVE_FEATHER_FALL {
                            guid: target,
                            counter: 0,
                        });
                    }
                    _ => {
                        p.outbound.send(&SMSG_MOVE_SET_HOVER {
                            guid: target,
                            counter: 0,
                        });
                    }
                }
            }
        }
        AURA_FAR_SIGHT | AURA_BIND_SIGHT => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_guid(PLAYER_FARSIGHT, h.caster);
            }
        }
        AURA_MOD_POSSESS | AURA_MOD_CHARM => {
            crate::game::map::pets::charmed(map, h.caster, target, kind == AURA_MOD_POSSESS);
        }
        AURA_ADD_FLAT_MODIFIER | AURA_ADD_PCT_MODIFIER => {
            crate::game::map::spells::mods::changed(
                map,
                target,
                a.mask,
                a.misc.max(0) as usize,
                kind == AURA_ADD_PCT_MODIFIER,
            );
        }
        AURA_MOD_TOTAL_THREAT => total_threat(map, target, a.amount * h.stacks.max(1) as i32),
        AURA_MOD_UNATTACKABLE | AURA_AURAS_VISIBLE => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, unit_flag_of(kind), true);
            }
        }
        AURA_GHOST => {}
        _ => {}
    }
}

/// An aura effect ended: undo what it did.
fn ended(map: &mut Map, target: Guid, h: &Holder, a: &Aura) {
    let still = |map: &Map, k: u32| auras(map, target).is_some_and(|x| x.has(k));
    match a.kind {
        AURA_MOD_STUN | AURA_MOD_ROOT => {
            let stunned = still(map, AURA_MOD_STUN);
            let rooted = stunned || still(map, AURA_MOD_ROOT);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_STUNNED,
                    stunned,
                );
            }
            if !rooted && let Some(p) = map.players.get(&target) {
                p.outbound.send(&SMSG_FORCE_MOVE_UNROOT {
                    guid: target,
                    counter: 0,
                });
            }
        }
        AURA_MOD_FEAR if !still(map, AURA_MOD_FEAR) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FLEEING, false);
            }
        }
        AURA_MOD_CONFUSE if !still(map, AURA_MOD_CONFUSE) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_CONFUSED, false);
            }
        }
        AURA_MOD_SILENCE | AURA_MOD_PACIFY_SILENCE
            if !still(map, AURA_MOD_SILENCE) && !still(map, AURA_MOD_PACIFY_SILENCE) =>
        {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SILENCED, false);
            }
        }
        AURA_MOD_PACIFY if !still(map, AURA_MOD_PACIFY) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PACIFIED, false);
            }
        }
        AURA_MOD_SHAPESHIFT if !still(map, AURA_MOD_SHAPESHIFT) => {
            shapeshift(map, target, 0, h.spell);
        }
        AURA_TRANSFORM if !still(map, AURA_TRANSFORM) => {
            if let Some(f) = map.unit_fields_mut(target) {
                let native = f.get_u32(UNIT_FIELD_NATIVEDISPLAYID);
                f.set_u32(UNIT_FIELD_DISPLAYID, native);
            }
        }
        AURA_MOUNTED if !still(map, AURA_MOUNTED) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 0);
            }
        }
        AURA_TRACK_CREATURES if !still(map, AURA_TRACK_CREATURES) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(PLAYER_TRACK_CREATURES, 0);
            }
        }
        AURA_TRACK_RESOURCES if !still(map, AURA_TRACK_RESOURCES) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(PLAYER_TRACK_RESOURCES, 0);
            }
        }
        AURA_MOD_SCALE => {
            if let Some(f) = map.unit_fields_mut(target) {
                let s = f.get_f32(OBJECT_FIELD_SCALE_X);
                f.set_f32(
                    OBJECT_FIELD_SCALE_X,
                    s * 100.0 / (100 + a.amount).max(1) as f32,
                );
            }
        }
        AURA_MOD_DISARM if !still(map, AURA_MOD_DISARM) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_DISARMED, false);
            }
        }
        AURA_FEIGN_DEATH if !still(map, AURA_FEIGN_DEATH) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FEIGN_DEATH, false);
                f.set_flag(UNIT_DYNAMIC_FLAGS, 0x20, false);
            }
        }
        AURA_MOD_TAUNT => {
            crate::game::map::ai::taunt_ended(map, target, h.caster);
        }
        AURA_MOD_STEALTH | AURA_MOD_INVISIBILITY => {
            if a.kind == AURA_MOD_STEALTH
                && !still(map, AURA_MOD_STEALTH)
                && let Some(f) = map.unit_fields_mut(target)
            {
                f.set_byte(UNIT_FIELD_BYTES_1, 2, f.get_byte(UNIT_FIELD_BYTES_1, 2) & !0x02);
            }
            crate::game::map::visibility::stealth_changed(map, target);
        }
        AURA_WATER_WALK if !still(map, AURA_WATER_WALK) => {
            if let Some(p) = map.players.get(&target) {
                p.outbound.send(&SMSG_MOVE_LAND_WALK {
                    guid: target,
                    counter: 0,
                });
            }
        }
        AURA_FEATHER_FALL if !still(map, AURA_FEATHER_FALL) => {
            if let Some(p) = map.players.get(&target) {
                p.outbound.send(&SMSG_MOVE_NORMAL_FALL {
                    guid: target,
                    counter: 0,
                });
            }
        }
        AURA_HOVER if !still(map, AURA_HOVER) => {
            if let Some(p) = map.players.get(&target) {
                p.outbound.send(&SMSG_MOVE_UNSET_HOVER {
                    guid: target,
                    counter: 0,
                });
            }
        }
        AURA_FAR_SIGHT | AURA_BIND_SIGHT => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_guid(PLAYER_FARSIGHT, Guid::zero());
            }
        }
        AURA_MOD_POSSESS | AURA_MOD_CHARM => {
            crate::game::map::pets::uncharmed(map, h.caster, target);
        }
        AURA_ADD_FLAT_MODIFIER | AURA_ADD_PCT_MODIFIER => {
            crate::game::map::spells::mods::changed(
                map,
                target,
                a.mask,
                a.misc.max(0) as usize,
                a.kind == AURA_ADD_PCT_MODIFIER,
            );
        }
        AURA_MOD_TOTAL_THREAT => total_threat(map, target, -(a.amount * h.stacks.max(1) as i32)),
        AURA_MOD_UNATTACKABLE | AURA_AURAS_VISIBLE if !still(map, a.kind) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, unit_flag_of(a.kind), false);
            }
        }
        _ => {}
    }
}

/// A unit's threat on every creature fighting it changes by `amount`, for
/// as long as the aura holds (Fade).
fn total_threat(map: &mut Map, target: Guid, amount: i32) {
    for (_, c) in map.creatures.iter_mut() {
        if let Some((_, t)) = c.threat.iter_mut().find(|(g, _)| *g == target) {
            *t = (*t + amount as f32).max(0.0);
        }
    }
}

/// The unit flag an aura sets, as Thistle Tea's unit sync does.
fn unit_flag_of(kind: u32) -> u32 {
    match kind {
        AURA_MOD_UNATTACKABLE => 0x0001_0000,
        _ => 0x0800_0000,
    }
}

/// A holder came off: what its going does.
fn removed(map: &mut Map, target: Guid, h: &Holder) {
    if h.caster == target {
        crate::game::map::spells::cooldowns::aura_gone(map, target, h.spell);
    }
    // A channel's aura gone ends the channel.
    if let Some(c) = map.caster(h.caster)
        && let Some(casting) = &c.casting
        && casting.spell == h.spell
        && casting.channel_until.is_some()
        && h.caster != target
        && !h.positive
    {
        let caster = h.caster;
        crate::game::map::spells::interrupt(map, caster);
    }
}

pub const UNIT_FLAG_SILENCED: u32 = 0x0000_2000;
pub const UNIT_FLAG_PACIFIED: u32 = 0x0002_0000;
pub const UNIT_FLAG_DISARMED: u32 = 0x0020_0000;
pub const UNIT_FLAG_CONFUSED: u32 = 0x0040_0000;
pub const UNIT_FLAG_FLEEING: u32 = 0x0080_0000;
pub const UNIT_FLAG_FEIGN_DEATH: u32 = 0x2000_0000;

/// Shapeshift forms.
pub const FORM_CAT: u8 = 1;
pub const FORM_TREE: u8 = 2;
pub const FORM_TRAVEL: u8 = 3;
pub const FORM_AQUA: u8 = 4;
pub const FORM_BEAR: u8 = 5;
pub const FORM_DIREBEAR: u8 = 8;
pub const FORM_STEALTH: u8 = 30;
pub const FORM_MOONKIN: u8 = 31;
pub const FORM_BATTLE: u8 = 17;
pub const FORM_DEFENSIVE: u8 = 18;
pub const FORM_BERSERKER: u8 = 19;

/// The passives a form brings (Thistle Tea's `@shapeshift_passives`).
fn form_passives(form: u8) -> &'static [u32] {
    match form {
        FORM_CAT => &[3025],
        FORM_TRAVEL => &[5419],
        FORM_AQUA => &[5421],
        FORM_BEAR => &[1178, 21178],
        FORM_DIREBEAR => &[9635, 21178],
        FORM_BATTLE => &[21156],
        FORM_DEFENSIVE => &[7376],
        FORM_BERSERKER => &[7381],
        FORM_MOONKIN => &[24905],
        _ => &[],
    }
}

/// A unit takes a form (0: its own): the form byte, a druid's power, the
/// form's passives, and auras a shapeshift breaks.
fn shapeshift(map: &mut Map, target: Guid, form: u8, by: u32) {
    let before = map
        .unit_fields(target)
        .map(|f| f.get_byte(UNIT_FIELD_BYTES_1, 2))
        .unwrap_or(0);
    if let Some(f) = map.unit_fields_mut(target) {
        f.set_byte(UNIT_FIELD_BYTES_1, 2, form);
    }
    for s in form_passives(before) {
        remove_spell(map, target, *s, Some(target));
    }
    let data = map.realm.data.clone();
    for s in form_passives(form) {
        if let Some(t) = data.spells.get(*s) {
            let effects: Vec<(usize, i32)> = (0..3)
                .filter(|i| t.effects[*i].effect == 6 && t.effects[*i].aura != 0)
                .map(|i| (i, crate::game::map::spells::points(t, i, 1)))
                .collect();
            let _ = apply_spell(map, target, target, t, &effects, Context::default());
        }
    }
    if let Some(p) = map.players.get_mut(&target)
        && p.row.class == 11
    {
        let power = match form {
            FORM_CAT => 3u8,
            FORM_BEAR | FORM_DIREBEAR => 1,
            _ => 0,
        };
        p.fields.set_byte(UNIT_FIELD_BYTES_0, 3, power);
        if power == 1 {
            p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + 1, 1000);
            p.fields.set_u32(UNIT_FIELD_POWER1 + 1, 0);
        }
        if power == 3 {
            p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + 3, 100);
            p.fields.set_u32(UNIT_FIELD_POWER1 + 3, 0);
        }
    }
    if form != before {
        crate::game::map::spells::scripts::shapeshifted(map, target, before, form);
    }
    if form != 0 && form != before {
        // Entering a form breaks what shapeshifting cancels, but the form.
        remove_where(map, target, |h| {
            h.spell != by && h.interrupt_flags & INTERRUPT_SHAPESHIFT != 0
        });
        // Druid forms shake off roots and snares.
        if matches!(form, 1..=5 | 8 | 31) {
            remove_where(map, target, |h| {
                !h.positive
                    && (h.has(AURA_MOD_ROOT) || h.has(AURA_MOD_DECREASE_SPEED))
                    && !matches!(h.mechanic, 1 | 2 | 5 | 8 | 12 | 13 | 18 | 20 | 23 | 24 | 27 | 30)
            });
        }
    }
}

/// The unit's form.
pub fn form(map: &Map, g: Guid) -> u8 {
    map.unit_fields(g)
        .map(|f| f.get_byte(UNIT_FIELD_BYTES_1, 2))
        .unwrap_or(0)
}

/// Recompute a unit's stats and speed with its auras.
pub fn recalc(map: &mut Map, target: Guid) {
    crate::game::map::derive::unit(map, target);
}

/// Expire auras, and tick the periodic ones.
pub fn update(map: &mut Map) {
    let now = map.now;
    let mut units: Vec<Guid> = map
        .players
        .values()
        .filter(|p| !p.auras.is_empty())
        .map(|p| p.guid)
        .collect();
    units.extend(
        map.creatures
            .iter()
            .filter(|(_, c)| !c.auras.is_empty())
            .map(|(_, c)| c.guid),
    );
    for g in units {
        let Some(a) = auras(map, g) else {
            continue;
        };
        let expired = a
            .holders
            .iter()
            .any(|h| h.expires.is_some_and(|t| t <= now));
        let ticks: Vec<(Holder, Aura)> = a
            .effects()
            .filter(|(_, x)| x.next_tick.is_some_and(|t| t <= now))
            .map(|(h, x)| (h.clone(), x.clone()))
            .collect();
        for (h, t) in ticks {
            if let Some(a) = auras_mut(map, g)
                && let Some(x) = a
                    .holders
                    .iter_mut()
                    .find(|x| x.spell == h.spell && x.caster == h.caster && x.item == h.item)
                    .and_then(|x| x.effects.iter_mut().find(|e| e.effect == t.effect))
                && let Some(p) = x.period
            {
                x.next_tick = Some(now + p);
                x.ticks += 1;
            }
            tick(map, g, &h, &t);
        }
        if expired {
            remove_where(map, g, |h| h.expires.is_some_and(|t| t <= now));
        }
    }
}

/// Run the ticks of `spell` from `caster` on `target` that are due now: a
/// channel's last tick, as the channel ends.
pub fn tick_due(map: &mut Map, target: Guid, spell: u32, caster: Guid) {
    let now = map.now;
    let due: Vec<(Holder, Aura)> = auras(map, target)
        .map(|a| {
            a.effects()
                .filter(|(h, x)| {
                    h.spell == spell && h.caster == caster && x.next_tick.is_some_and(|t| t <= now)
                })
                .map(|(h, x)| (h.clone(), x.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (h, x) in due {
        tick(map, target, &h, &x);
    }
}

fn tick(map: &mut Map, target: Guid, h: &Holder, a: &Aura) {
    let alive = map.unit_view(target).is_some_and(|u| u.alive);
    if !alive {
        return;
    }
    let stacks = h.stacks.max(1) as i32;
    match a.kind {
        AURA_PERIODIC_DAMAGE | AURA_PERIODIC_LEECH | AURA_PERIODIC_DAMAGE_PERCENT => {
            let amount = if a.kind == AURA_PERIODIC_DAMAGE_PERCENT {
                let max = map
                    .unit_fields(target)
                    .map(|f| f.get_u32(UNIT_FIELD_MAXHEALTH))
                    .unwrap_or(0);
                (max as i32 * a.amount * stacks / 100).max(0) as u32
            } else {
                (a.amount * stacks).max(0) as u32
            };
            crate::game::map::spells::periodic_damage(map, h, target, amount, a.kind == AURA_PERIODIC_LEECH);
        }
        AURA_PERIODIC_HEAL | AURA_OBS_MOD_HEALTH | AURA_MOD_REGEN => {
            let amount = if a.kind == AURA_OBS_MOD_HEALTH {
                let max = map
                    .unit_fields(target)
                    .map(|f| f.get_u32(UNIT_FIELD_MAXHEALTH))
                    .unwrap_or(0);
                max * (a.amount * stacks).max(0) as u32 / 100
            } else {
                (a.amount * stacks).max(0) as u32
            };
            crate::game::map::spells::heal(map, h.caster, target, h.spell, amount, true);
        }
        AURA_PERIODIC_HEALTH_FUNNEL => {
            // The caster pays health for the target's.
            let cost = (a.amount * stacks).max(0) as u32;
            let paid = map
                .unit_fields(h.caster)
                .map(|f| f.get_u32(UNIT_FIELD_HEALTH) > cost)
                .unwrap_or(false);
            if paid {
                crate::game::map::combat::deal_damage(map, h.caster, h.caster, cost);
                let gain = (cost as f32
                    * map
                        .realm
                        .data
                        .spells
                        .get(h.spell)
                        .map_or(1.0, |s| s.effects[a.effect as usize].multiple_value.max(1.0)))
                    as u32;
                crate::game::map::spells::heal(map, h.caster, target, h.spell, gain, true);
            }
        }
        AURA_PERIODIC_ENERGIZE | AURA_OBS_MOD_MANA | AURA_MOD_POWER_REGEN => {
            if let Some(f) = map.unit_fields_mut(target) {
                let power = a.misc.clamp(0, 4) as u16;
                let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
                let add = if a.kind == AURA_OBS_MOD_MANA {
                    max * (a.amount * stacks).max(0) as u32 / 100
                } else {
                    (a.amount * stacks).max(0) as u32
                };
                let now = f.get_u32(UNIT_FIELD_POWER1 + power);
                f.set_u32(UNIT_FIELD_POWER1 + power, (now + add).min(max));
            }
        }
        AURA_PERIODIC_MANA_LEECH | AURA_POWER_BURN_MANA => {
            let take = (a.amount * stacks).max(0) as u32;
            let power = a.misc.clamp(0, 4) as u16;
            let taken = map
                .unit_fields_mut(target)
                .map(|f| {
                    let have = f.get_u32(UNIT_FIELD_POWER1 + power);
                    let t = take.min(have);
                    f.set_u32(UNIT_FIELD_POWER1 + power, have - t);
                    t
                })
                .unwrap_or(0);
            let mult = map
                .realm
                .data
                .spells
                .get(h.spell)
                .map_or(1.0, |s| s.effects[a.effect as usize].multiple_value);
            if a.kind == AURA_PERIODIC_MANA_LEECH {
                let gain = (taken as f32 * mult.max(0.0)) as u32;
                if let Some(f) = map.unit_fields_mut(h.caster) {
                    let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
                    let now = f.get_u32(UNIT_FIELD_POWER1 + power);
                    f.set_u32(UNIT_FIELD_POWER1 + power, (now + gain).min(max));
                }
            } else {
                let damage = (taken as f32 * mult.max(0.0)) as u32;
                crate::game::map::spells::periodic_damage(map, h, target, damage, false);
            }
        }
        AURA_PERIODIC_TRIGGER_SPELL if a.trigger != 0 => {
            // A channel's trigger aims at the channel's target.
            let aim = if h.caster == target {
                map.caster(h.caster)
                    .and_then(|c| c.casting.as_ref())
                    .and_then(|c| c.targets.unit)
                    .filter(|u| *u != target)
                    .or_else(|| {
                        map.unit_fields(target)
                            .map(|f| f.get_guid(UNIT_FIELD_TARGET))
                            .filter(|g| !g.is_zero())
                    })
                    .unwrap_or(target)
            } else {
                target
            };
            crate::game::map::spells::cast_triggered(map, h.caster, a.trigger, Some(aim));
        }
        _ => {}
    }
}

/// Damage of `school` a unit takes, less what its absorb auras and mana
/// shields soak: (what's left, what's absorbed).
pub fn absorb(map: &mut Map, target: Guid, damage: u32, school: u32) -> (u32, u32) {
    let Some(a) = auras_mut(map, target) else {
        return (damage, 0);
    };
    let mut left = damage as i32;
    let mut used_up = Vec::new();
    let mut mana_cost = 0u32;
    for h in a.holders.iter_mut() {
        for x in h.effects.iter_mut() {
            if left <= 0 {
                break;
            }
            if !matches!(x.kind, AURA_SCHOOL_ABSORB | AURA_MANA_SHIELD)
                || (x.misc as u32) & school == 0
            {
                continue;
            }
            let soak = x.left.min(left).max(0);
            x.left -= soak;
            left -= soak;
            if x.kind == AURA_MANA_SHIELD {
                mana_cost += soak as u32 * 2;
            }
            if x.left <= 0 {
                used_up.push((h.spell, h.caster));
            }
        }
    }
    if mana_cost > 0
        && let Some(f) = map.unit_fields_mut(target)
    {
        let mana = f.get_u32(UNIT_FIELD_POWER1);
        f.set_u32(UNIT_FIELD_POWER1, mana.saturating_sub(mana_cost));
    }
    for (spell, caster) in used_up {
        remove_spell(map, target, spell, Some(caster));
    }
    let left = left.max(0) as u32;
    (left, damage - left)
}

/// Whether a unit is held: stunned, feared or confused, or (for `casting`)
/// silenced.
pub fn held(map: &Map, g: Guid, casting: bool) -> bool {
    auras(map, g).is_some_and(|a| {
        a.has(AURA_MOD_STUN)
            || a.has(AURA_MOD_FEAR)
            || a.has(AURA_MOD_CONFUSE)
            || (casting && (a.has(AURA_MOD_SILENCE) || a.has(AURA_MOD_PACIFY_SILENCE)))
    })
}

/// `CMSG_CANCEL_AURA`: a player takes a helpful aura off.
pub fn cancel(map: &mut Map, player: Guid, spell: u32) {
    let data = map.realm.data.clone();
    let cancelable = map.players.get(&player).is_some_and(|p| {
        p.auras
            .holders
            .iter()
            .any(|h| h.spell == spell && h.positive && !h.passive)
    }) && !data
        .spells
        .get(spell)
        .is_some_and(|s| s.attr(0, sd::ATTR_CANT_CANCEL));
    if cancelable {
        remove_where(map, player, |h| h.spell == spell && !h.passive);
    }
}

/// On death every aura but the passive and death-persistent ones goes, and
/// so does the cast.
pub fn clear_on_death(map: &mut Map, g: Guid) {
    crate::game::map::spells::interrupt(map, g);
    let data = map.realm.data.clone();
    remove_where(map, g, |h| {
        !h.passive
            && !data
                .spells
                .get(h.spell)
                .is_some_and(|s| s.attr(3, sd::ATTR_EX3_DEATH_PERSISTENT))
    });
}

/// The holders a dispel of `dispel` can take, helpful (`friendly` target)
/// or harmful.
pub fn dispellable(map: &Map, target: Guid, dispel: u32, friendly: bool) -> Vec<(u32, Guid)> {
    auras(map, target)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| !h.passive && h.positive != friendly)
                .filter(|h| dispel == 0 || h.dispel == dispel || (dispel == 1 && h.dispel == 1))
                .filter(|h| h.dispel != 0)
                .map(|h| (h.spell, h.caster))
                .collect()
        })
        .unwrap_or_default()
}
