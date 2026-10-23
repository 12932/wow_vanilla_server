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

use crate::game::conn::Order;
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
/// A charm of many (Chains of Kel'Thuzad): a charm all the same (mangos's
/// `HandleAoECharm`, `TakeCharmOf`).
pub const AURA_AOE_CHARM: u32 = 177;
pub const AURA_MOD_SPEED_NOT_STACK: u32 = 171;
pub const AURA_MOD_MOUNTED_SPEED_NOT_STACK: u32 = 172;
pub const AURA_ALLOW_CHAMPION_SPELLS: u32 = 173;
pub const AURA_MOD_SPELL_DAMAGE_OF_STAT_PERCENT: u32 = 174;
pub const AURA_MOD_SPELL_HEALING_OF_STAT_PERCENT: u32 = 175;
/// A change to the crit chance of harmful spells of the schools of its
/// mask cast at the unit (Winter's Chill's 2% a stack).
pub const AURA_MOD_ATTACKER_SPELL_CRIT_CHANCE: u32 = 179;
/// Spell damage against creature types (`misc` their mask).
pub const AURA_MOD_FLAT_SPELL_DAMAGE_VERSUS: u32 = 180;
/// The threat of a critical hit of the schools of its mask (28746's).
pub const AURA_MOD_CRITICAL_THREAT: u32 = 183;
/// Changes to the hit and crit chance of those who attack the unit.
pub const AURA_MOD_ATTACKER_MELEE_HIT_CHANCE: u32 = 184;
pub const AURA_MOD_ATTACKER_RANGED_HIT_CHANCE: u32 = 185;
/// A change to the chance of spells of the schools of its mask to hit the
/// unit (Heightened Senses' 2% less).
pub const AURA_MOD_ATTACKER_SPELL_HIT_CHANCE: u32 = 186;
pub const AURA_MOD_ATTACKER_MELEE_CRIT_CHANCE: u32 = 187;
pub const AURA_MOD_ATTACKER_RANGED_CRIT_CHANCE: u32 = 188;
/// vmangos's own: the trigger spell's auras held while this one lasts
/// (`spell_effect_mod` gives Barkskin one, for "Barkskin Effect (DND)").
pub const AURA_LINKED: u32 = 192;

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
    (aura <= 175 && !matches!(aura, 46 | 48 | 63 | 119 | 164))
        || aura == AURA_MOD_FLAT_SPELL_DAMAGE_VERSUS
        || aura == AURA_LINKED
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
    /// `EffectItemType`: the spells a modifier or trigger affects (its
    /// family flags), or an item.
    pub mask: u64,
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
    GeneralsWarcry,
}

impl Exclusive {
    /// Whether one of each caster's may be on a unit, rather than one in
    /// all. A paladin's aura is one of each paladin's: two paladins'
    /// different auras stand together, as mangos-classic's
    /// `SpellStacker::IsStackableAuraEffect` compares only like effects
    /// (the same aura from two is the rank chain's and the groups').
    fn per_caster(self) -> bool {
        matches!(
            self,
            Exclusive::PaladinBlessing
                | Exclusive::WarlockCurse
                | Exclusive::HunterSting
                | Exclusive::PaladinJudgement
                | Exclusive::PaladinAura
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

/// The paladin's blessings one of each paladin's may be on a unit:
/// mangos-classic's spell group 7 ("Blessing", `UNIQUE_PER_CASTER`), its
/// 1.12 spells. Might, Wisdom, Kings, Salvation, Light and Sanctuary, their
/// greater blessings, Protection and Sacrifice; not Freedom, which the
/// group leaves out though its family flags are a blessing's, so it stands
/// beside its paladin's Might (nor the creatures' Improved Blessing of
/// Protection and Shield of Rajaxx).
const BLESSINGS: [u32; 35] = [
    1022, 5599, 10_278, // Protection
    1038, 25_895, // Salvation
    6940, 20_729, // Sacrifice
    19_740, 19_834, 19_835, 19_836, 19_837, 19_838, 25_291, 25_782, 25_916, // Might
    19_742, 19_850, 19_852, 19_853, 19_854, 25_290, 25_894, 25_918, // Wisdom
    19_977, 19_978, 19_979, 25_890, // Light
    20_217, 25_898, // Kings
    20_911, 20_912, 20_913, 20_914, 25_899, // Sanctuary
];

/// The exclusive kind of a spell, as Thistle Tea's `exclusive_category`
/// works it out (the blessings as mangos-classic groups them).
pub fn exclusive(s: &SpellTemplate, elixir: Option<u32>) -> Option<Exclusive> {
    if matches!(s.id, 28418..=28420) {
        return Some(Exclusive::GeneralsWarcry);
    }
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
        || (s.family_name == FAMILY_HUNTER
            && s.spell_icon != 0
            && active_icon(s) == 122
            && s.id != 75)
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
        if BLESSINGS.contains(&s.id) {
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
    // Slows aren't exclusive, as Thistle Tea's `Slow` makes them: snares
    // stack, each on as long as it lasts, and the strongest slows
    // (mangos's `UpdateSpeed`, the user's #27); attack-speed slows stack
    // and multiply (mangos-classic's `ApplyAttackTimePercentMod`, #30).
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
    /// When crowd control may break early.
    pub heartbeat: Option<crate::game::map::spells::heartbeat::Heartbeat>,
    /// From a triggered spell (worth less when the bar is full).
    pub triggered: bool,
    /// When it was put on (or last refreshed): what a duel did goes at its
    /// end (mangos's `GetAuraApplyTime`).
    pub applied: Option<Instant>,
    /// The aura (its spell and caster) whose trigger put this lasting one
    /// on, and with which it goes (Thistle Tea's `TriggeredLifetime`: the
    /// Badge of the Swarmguard's Insight of the Qiraji, Jom Gabbar's).
    pub source: Option<(u32, Guid)>,
    /// The diminishing returns group the cast put it on in (`spells::Dr`
    /// as a number), if harmful and of one: a later aura of its group
    /// takes its place (mangos-classic's `getDiminishGroup` in
    /// `RemoveNoStackAurasDueToAuraHolder`).
    pub dr: Option<u8>,
    /// Not kept across a logout though it lasts (mangos's
    /// `IsSaveToDbHolder`): a single-target aura, or an Echoes of
    /// Lordaeron world buff.
    pub unsaved: bool,
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
    /// The unit's own size, bounding radius and combat reach, kept while a
    /// transform or a growth changes them, to come back to (Thistle Tea's
    /// `base_scale_x`, `base_bounding_radius` and `base_combat_reach`).
    pub native_size: Option<[f32; 3]>,
    /// A creature's own weapons (its virtual item fields), kept while a
    /// transform puts its creature's on.
    pub native_weapons: Option<[u32; 9]>,
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

    /// Whether `spell`, a school `mask`, is stopped by an immunity.
    pub fn school_immune(&self, mask: u32) -> bool {
        self.of(AURA_SCHOOL_IMMUNITY)
            .chain(self.of(AURA_DAMAGE_IMMUNITY))
            .any(|(_, a)| (a.misc as u32) & mask != 0 && mask != 0)
    }

    /// Whether a school `mask` is stopped by an immunity of `kind` alone
    /// (`AURA_SCHOOL_IMMUNITY`, which a spell may pierce, or
    /// `AURA_DAMAGE_IMMUNITY`, which it may not).
    pub fn immune_by(&self, kind: u32, mask: u32) -> bool {
        self.of(kind)
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

    /// Whether auras of `kind` are kept off (`SPELL_AURA_STATE_IMMUNITY`:
    /// Thickskull's stuns, Fearless's fears, a boss's taunts).
    pub fn state_immune(&self, kind: u32) -> bool {
        kind != 0
            && self
                .of(AURA_STATE_IMMUNITY)
                .any(|(_, a)| a.misc as u32 == kind)
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
/// `SPELL_EFFECT_INSTAKILL`.
const EFFECT_INSTAKILL: u32 = 1;

/// Shadow Trance's and Netherwind Focus's.
const ONE_CAST_MODIFIERS: [u32; 2] = [17_941, 22_008];

/// The charges an aura of `spell` starts with: its own, but one for the
/// modifiers spent by the next cast though their data has none (mangos's
/// `HandleAddModifier`: Shadow Trance's instant Shadow Bolt, Netherwind
/// Focus's instant spell).
pub fn own_charges(spell: &SpellTemplate) -> u32 {
    if ONE_CAST_MODIFIERS.contains(&spell.id) {
        1
    } else {
        spell.proc_charges
    }
}

pub fn positive(s: &SpellTemplate) -> bool {
    // vmangos's own word on it first, its positive over its negative (as
    // Thistle Tea's `harmful_instakill?`; no spell has both).
    if s.custom(sd::CUSTOM_POSITIVE) {
        return true;
    }
    if s.custom(sd::CUSTOM_NEGATIVE) {
        return false;
    }
    if s.attr(0, sd::ATTR_DEBUFF) {
        return false;
    }
    !s.effects.iter().any(|e| harmful_effect(s, e))
}

/// Whether an implicit target is a friend's: mangos's `TARGET_HELPFUL`
/// (the caster, its pet or master, a friend, friends about a place or in
/// a cone, a chain heal's), `TARGET_PARTY` and `TARGET_GROUP` (a party or
/// raid member, those about the caster or the target).
pub fn friendly_target(t: u32) -> bool {
    matches!(
        t,
        1 | 3 | 5 | 14 | 20 | 21 | 27 | 29..=31 | 33..=35 | 37 | 45 | 56..=59 | 61 | 62
    )
}

/// Whether an implicit target is a foe's: mangos's `TARGET_HARMFUL`.
pub fn hostile_target(t: u32) -> bool {
    matches!(t, 2 | 6 | 13 | 15 | 16 | 24 | 28 | 36 | 54)
}

/// Whether effect `e` of `s` harms: damage, a harmful aura, a buff's
/// aura turned against its target, a hostile target, an instant kill.
///
/// An aura is judged by its target first, as mangos's `IsPositiveEffect`
/// does: one put on a friend (a friendly implicit target, and no hostile
/// one) helps whatever it does, unless the spell says it is a debuff
/// (`positive`'s vmangos negative and `SPELL_ATTR_AURA_IS_DEBUFF`).
/// Blessing of Protection's pacify and Divine Shield's are a friend's
/// shield, not an attack.
pub fn harmful_effect(s: &SpellTemplate, e: &crate::game::data::spells::SpellEffect) -> bool {
    let on_a_friend = (friendly_target(e.target_a) || friendly_target(e.target_b))
        && !hostile_target(e.target_a)
        && !hostile_target(e.target_b);
    {
        matches!(e.effect, 2 | 9 | 17 | 31 | 58 | 62 | 121)
            || !on_a_friend && matches!(
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
                    | AURA_AOE_CHARM
                    | AURA_MOD_POSSESS
                    | AURA_MOD_TAUNT
                    | AURA_MOD_STALKED
                    | AURA_MOD_DISARM
                    | AURA_TRANSFORM
                    | AURA_MOD_DETECT_RANGE
            )
            || !on_a_friend && (matches!(
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
            || !on_a_friend && (matches!(
                e.aura,
                AURA_MOD_DAMAGE_TAKEN | AURA_MOD_DAMAGE_PERCENT_TAKEN | AURA_MOD_MELEE_DAMAGE_TAKEN
            ) && e.base_points + e.base_dice > 0)
            || matches!(e.target_a, 6 | 15 | 16 | 24 | 28) && e.effect != 0
            // The caster's place is neither friend nor foe (mangos's
            // `TARGET_LOCATION_CASTER_SRC`, neutral): its area says which,
            // so the Healing Auras about their ward help (Thistle Tea's
            // "healing wards select friends").
            || e.target_a == 22 && matches!(e.target_b, 2 | 15 | 16 | 24 | 28 | 36 | 54) && e.effect != 0
            // An instant kill of anything but the caster alone (Death
            // Touch at any unit), but a Voidwalker's Sacrifice (Thistle
            // Tea's `harmful_instakill?`).
            || e.effect == EFFECT_INSTAKILL
                && !(e.target_a == 1 && e.target_b == 0)
                && !(s.family_name == 5 && s.family_flags & 0x0200_0000 != 0)
    }
}

/// The auras of `g`, mutably.
pub fn auras_mut(map: &mut Map, g: Guid) -> Option<&mut Auras> {
    if map.players.contains_key(&g) {
        return map.players.get_mut(&g).map(|p| &mut p.auras);
    }
    // Every aura a creature gets comes by here (see `Map::with_auras`).
    if map.creature_by_guid.contains_key(&g) {
        map.with_auras.insert(g);
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
    /// From a triggered spell.
    pub triggered: bool,
    /// A linked aura's spell: held unseen and as a passive, with no slot
    /// of its own, going with the aura that holds it (Thistle Tea's linked
    /// holders).
    pub linked: bool,
    /// The cast's activation-time modifiers (op 19), flat and percent: what
    /// they make of a periodic aura's interval.
    pub activation: (i32, i32),
    /// The cast's diminishing returns group (`Holder::dr`).
    pub dr: Option<u8>,
}

/// Whether an aura of `kind` ticks (Thistle Tea's `Effect.periodic?`).
pub fn ticks(kind: u32) -> bool {
    matches!(
        kind,
        AURA_PERIODIC_DAMAGE
            | AURA_PERIODIC_HEAL
            | AURA_OBS_MOD_HEALTH
            | AURA_OBS_MOD_MANA
            | AURA_PERIODIC_TRIGGER_SPELL
            | AURA_PERIODIC_ENERGIZE
            | AURA_PERIODIC_LEECH
            | AURA_PERIODIC_HEALTH_FUNNEL
            | AURA_PERIODIC_MANA_LEECH
            | AURA_PERIODIC_DAMAGE_PERCENT
            | AURA_POWER_BURN_MANA
    )
}

/// An aura effect's interval: its amplitude, or a second for a percentage
/// of mana that has none (mangos's `HandleAuraModTotalManaPercentRegen`),
/// or three for a regeneration of rage that has none (Anger Management's
/// 0 in the DBC; mangos's `HandleModPowerRegen` ticks it every 3000 ms).
/// A periodic aura's takes the activation-time modifiers `(flat, pct)`,
/// and is at least a millisecond (mangos's `Aura` with
/// `SPELLMOD_ACTIVATION_TIME`, Thistle Tea's `Modifiers.periodic_interval`).
pub fn interval(e: &sd::SpellEffect, activation: (i32, i32)) -> Option<Duration> {
    let amplitude = match e.amplitude {
        0 if e.aura == AURA_OBS_MOD_MANA => 1000,
        0 if e.aura == AURA_MOD_POWER_REGEN && e.misc_value == 1 => 3000,
        a => a,
    };
    if amplitude == 0 {
        return None;
    }
    let ms = if ticks(e.aura) {
        let (flat, pct) = activation;
        ((i64::from(amplitude) + i64::from(flat)) * i64::from((100 + pct).max(0)) / 100).max(1)
    } else {
        i64::from(amplitude)
    };
    Some(Duration::from_millis(ms as u64))
}

/// Why an aura couldn't be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// A stronger rank or a higher spell of its group is there.
    Bounced,
    Immune,
}

/// Put `spell`'s effect `index` on `target` (the tests' shorthand: a
/// spell's effects go on together, by [`apply_spell`]).
#[cfg(test)]
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
    if crate::game::map::item_scripts::instead_of_aura(map, caster, spell) {
        return Ok(());
    }
    let now = map.now;
    let data = map.realm.data.clone();
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    let duration = ctx
        .duration
        .unwrap_or_else(|| data.spells.duration(spell, level));
    let pos = positive(spell);
    let passive = spell.passive() || ctx.linked;
    let Some(existing) = auras(map, target) else {
        return Ok(());
    };
    // A creature's template's immunities: the spell's, and each effect's
    // own mechanic's, which keeps that effect off.
    if crate::game::map::creature_combat::spell_immune(map, target, caster, spell) {
        return Err(Refused::Immune);
    }
    let kept: Vec<(usize, i32)> = effects
        .iter()
        .copied()
        .filter(|(i, _)| {
            !crate::game::map::creature_combat::effect_immune(
                map,
                target,
                caster,
                spell.effects[*i].mechanic,
            )
        })
        .collect();
    if kept.is_empty() && !effects.is_empty() {
        return Err(Refused::Immune);
    }
    let effects = &kept[..];
    // Immunities: a harmful spell's mechanic or school.
    if !pos && !spell.attr(0, sd::ATTR_NO_IMMUNITIES) {
        let mechanic_immune = existing.mechanic_immune(spell.mechanic)
            || effects
                .iter()
                .any(|(i, _)| existing.mechanic_immune(spell.effects[*i].mechanic));
        if mechanic_immune
            || (!spell.attr(2, sd::ATTR_EX2_NO_SCHOOL_IMMUNITIES)
                && existing.school_immune(spell.school_mask()))
        {
            return Err(Refused::Immune);
        }
    }
    // Effects of an aura kind kept off don't go on, whoever the target
    // (mangos's `IsImmuneToSpellEffect`, Thistle Tea's state immunity on a
    // self-cast harmful aura); with none left the aura is refused.
    let kept_off: Vec<(usize, i32)>;
    let effects = if !spell.attr(0, sd::ATTR_NO_IMMUNITIES)
        && effects
            .iter()
            .any(|(i, _)| existing.state_immune(spell.effects[*i].aura))
    {
        kept_off = effects
            .iter()
            .copied()
            .filter(|(i, _)| !existing.state_immune(spell.effects[*i].aura))
            .collect();
        if kept_off.is_empty() {
            return Err(Refused::Immune);
        }
        &kept_off[..]
    } else {
        effects
    };
    // A dispel type kept off, helpful or not: Faerie Fire's keeps stealth
    // and invisibility off (mangos's `IsImmuneToSpell`).
    if !spell.attr(0, sd::ATTR_NO_IMMUNITIES) && existing.dispel_immune(spell.dispel) {
        return Err(Refused::Immune);
    }
    // A shield, a bandage or a bubble too soon after the last: Weakened
    // Soul, Recently Bandaged and Forbearance are immunities to its kind.
    if pos && matches!(spell.mechanic, 16 | 19 | 25) && existing.mechanic_immune(spell.mechanic) {
        return Err(Refused::Immune);
    }
    // A depleting trinket's bonus comes only with its trinket's aura.
    if crate::game::map::trinkets::orphan(existing, spell.id, caster) {
        return Err(Refused::Bounced);
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
        if blocked {
            return Err(Refused::Bounced);
        }
    }
    // A weaker spell of a group that keeps its highest (rule 3), helpful
    // or harmful: a lesser Demoralizing Shout under a Demoralizing Roar.
    if !passive && group_relation(&data, existing, spell) == Some(GroupRelation::Blocked) {
        return Err(Refused::Bounced);
    }
    let mut aura_effects = Vec::new();
    for &(index, amount) in effects {
        let e = spell.effects[index];
        let period =
            interval(&e, ctx.activation).or_else(|| crate::game::map::captured::period(spell.id));
        aura_effects.push(Aura {
            effect: index as u8,
            kind: e.aura,
            amount,
            misc: e.misc_value,
            trigger: e.trigger_spell,
            mask: e.item_type,
            period,
            next_tick: period.map(|p| {
                if crate::game::map::spells::class_scripts::immediate_periodic(spell.id) {
                    now
                } else {
                    now + p
                }
            }),
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
        charges: ctx.charges.unwrap_or_else(|| own_charges(spell)),
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
        heartbeat: crate::game::map::spells::heartbeat::prepare(
            map, caster, target, spell, pos, duration,
        ),
        triggered: ctx.triggered,
        applied: Some(now),
        // A lasting aura that an aura of the caster's on the unit triggers
        // (its proc's or its pulse's spell) lasts no longer than it: one
        // that runs out later, or itself goes with another. Not one put on
        // at its source's last moment (a pulse's at its end), which stays
        // (Thistle Tea's `TriggeredLifetime.source`).
        source: (duration.is_none() && !passive && !ctx.linked)
            .then(|| {
                existing.holders.iter().find(|s| {
                    s.caster == caster
                        && (s.expires.is_some_and(|t| t > now) || s.source.is_some())
                        && s.effects.iter().any(|x| {
                            matches!(
                                x.kind,
                                AURA_PROC_TRIGGER_SPELL | AURA_PERIODIC_TRIGGER_SPELL
                            ) && x.trigger == spell.id
                        })
                })
            })
            .flatten()
            .map(|s| (s.spell, s.caster)),
        dr: ctx.dr,
        unsaved: crate::game::map::spells::cooldowns::unsaved(spell),
    };
    let mut holder = holder;
    crate::game::map::trinkets::prepare(existing, &mut holder, spell);
    let visible = visible(spell) && !ctx.linked;
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
    // The same spell from the same caster and item: refreshed, and a stack
    // more.
    let same = |h: &Holder| {
        h.spell == spell.id
            && (h.caster == caster || spell.custom(sd::CUSTOM_ALLOW_STACK_BETWEEN_CASTERS))
            && h.item == ctx.item
    };
    let beat = existing.holders.iter().find(|h| same(h)).map(|old| {
        crate::game::map::spells::heartbeat::refreshed(
            map,
            caster,
            spell,
            old.heartbeat,
            holder.heartbeat,
            holder.duration,
        )
    });
    {
        let Some(a) = auras_mut(map, target) else {
            return Ok(());
        };
        if let Some(i) = a.holders.iter().position(same) {
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
            // Its proc's cooldown runs on (mangos keeps it on the unit, by
            // spell): a seal or a weapon's buff renewed doesn't proc at once.
            let proc_ready_at = old.proc_ready_at;
            *old = Holder {
                stacks,
                slot,
                proc_ready_at,
                ..holder.clone()
            };
            old.heartbeat = beat.flatten();
            // A stacking aura keeps its ticks' rhythm (and some others).
            if crate::game::map::spells::class_scripts::preserves_periodic_timer(spell) {
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
    // Taken off as any removal takes it off (`remove_where`): its returns
    // recover, a cooldown waiting for it starts, a channel it was ends.
    for h in gone {
        for x in &h.effects {
            ended(map, target, &h, x);
        }
        removed(map, target, &h);
    }
    // A slot for the new holder, found again: what the replaced ones' ends
    // did (a form's passives given back, a source's lasting auras taken
    // off, this one among them) moves the others.
    let Some(mut index) = auras(map, target).and_then(|a| {
        a.holders
            .iter()
            .rposition(|h| h.spell == spell.id && h.caster == caster && h.item == ctx.item)
    }) else {
        recalc(map, target);
        return Ok(());
    };
    if visible {
        let mut slot = {
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
                let range = if pos {
                    0..POSITIVE_SLOTS
                } else {
                    POSITIVE_SLOTS..MAX_SLOTS
                };
                range
                    .into_iter()
                    .find(|i| !a.slot_taken(*i) && !fields_taken[*i])
            })
        };
        // All slots of its kind taken: the one of least worth gives way.
        if slot.is_none()
            && let Some(i) = crate::game::map::spells::capacity::gives_way(map, target, pos, index)
        {
            if i == index {
                if let Some(a) = auras_mut(map, target) {
                    a.holders.remove(index);
                }
                // What it replaced is gone from the stats too.
                recalc(map, target);
                return Ok(());
            }
            let Some(old) = auras(map, target).map(|a| a.holders[i].clone()) else {
                return Ok(());
            };
            slot = old.slot;
            remove_where(map, target, |h| {
                h.spell == old.spell
                    && h.caster == old.caster
                    && h.item == old.item
                    && h.slot == old.slot
            });
            let Some(found) = auras(map, target).and_then(|a| {
                a.holders.iter().position(|h| {
                    h.spell == spell.id
                        && h.caster == caster
                        && h.item == ctx.item
                        && h.slot.is_none()
                })
            }) else {
                return Ok(());
            };
            index = found;
        }
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
    // A refresh renews the children it links, or makes again one taken off
    // (Thistle Tea's `Linked`); not one a link made, so a link back to its
    // parent ends there rather than going round.
    if let Some(h) = &h
        && refreshed.is_some()
        && !ctx.linked
    {
        for x in h.effects.iter().filter(|x| x.kind == AURA_LINKED) {
            linked(map, target, h, x.trigger);
        }
        // And what the scripts add with it (Bestial Wrath's immunities).
        crate::game::map::spells::class_scripts::aura_applied(map, target, h);
        // A transform refreshed is the newest put on, and lands again: a
        // Discombobulate again takes its target off the mount it got on
        // since (Thistle Tea's "only a new Discombobulate transformation
        // interrupts a mount").
        if let Some(x) = h.effects.iter().find(|x| x.kind == AURA_TRANSFORM) {
            sync_display(map, target);
            crate::game::map::spells::interrupts::transformed(map, target, h.spell, x.effect);
        }
    }
    if let Some(h) = h
        && refreshed.is_none()
    {
        for x in &h.effects {
            started(map, target, &h, x);
        }
        dummy_flags(map, target, spell.id, true);
        if spell.attr(1, sd::ATTR_EX_PREVENTS_ANIM) {
            body_held(map, target);
        }
        crate::game::map::trinkets::applied(map, target, spell.id, caster);
        crate::game::map::spells::class_scripts::aura_applied(map, target, &h);
        crate::game::map::spells::single_target::applied(map, caster, target, spell);
        crate::game::map::spells::immunity::applied(map, target, &h);
        // Food and drink seat the eater (mangos's `_AddSpellAuraHolder`).
        if spell.aura_interrupt_flags & INTERRUPT_NOT_SEATED != 0 {
            crate::game::map::chat::sit_for_aura(map, target);
        }
    }
    recalc(map, target);
    crate::game::map::spell_areas::prerequisite_changed(map, target, spell.id);
    // Helping a flagged player flags the helper.
    if pos && caster != target {
        crate::game::map::combat::assist_contact(map, caster, target, spell);
    }
    Ok(())
}

/// Whether an aura shows on the unit's bar. A party aura shows on all it
/// is on, its caster too, passive or not (mangos's `IsNeedVisibleSlot`:
/// "passive party auras shown even on caster"; the user's call, #57).
pub fn visible(s: &SpellTemplate) -> bool {
    if s.attributes[0] == sd::ATTR_HIDDEN && s.duration_index == 21 {
        return false;
    }
    if s.effects.iter().any(|e| e.effect == 35 && e.aura != 0) {
        return true;
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
    if s.channeled()
        || s.attr(3, sd::ATTR_EX3_DOT_STACKING)
        || s.custom(sd::CUSTOM_SEPARATE_AURA_PER_CASTER)
    {
        return true;
    }
    effects.iter().all(|(i, _)| {
        !h.effects.iter().any(|x| x.effect as usize == *i) || PERSONAL.contains(&s.effects[*i].aura)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupRelation {
    Replaces,
    Blocked,
}

/// How `spell` stands to the spells of its groups already on the unit:
/// rule 1 replaces, rule 3 keeps the higher in the group's order
/// (`group_spell_id`).
fn group_relation(
    data: &crate::game::data::GameData,
    a: &Auras,
    spell: &SpellTemplate,
) -> Option<GroupRelation> {
    // Any holder that blocks it refuses it whole, whatever another group
    // would have it replace (Thistle Tea's atomic rejection).
    let mut out = None;
    for h in &a.holders {
        match group_relation_to(data, spell, h) {
            Some(GroupRelation::Blocked) => return Some(GroupRelation::Blocked),
            Some(r) => out = Some(r),
            None => {}
        }
    }
    out
}

/// How `spell` stands to holder `h` by their first common group with a
/// rule (vmangos's `spell_group_stack_rules`): none for the same spell, a
/// rank of its own kind, a passive or hidden holder (what stands apart
/// from the groups, as Thistle Tea's exclusivity keeps them), or a spell
/// and the one it triggers.
fn group_relation_to(
    data: &crate::game::data::GameData,
    spell: &SpellTemplate,
    h: &Holder,
) -> Option<GroupRelation> {
    groups_relation_to(data, spell, h).or_else(|| multirank_relation_to(data, spell, h))
}

/// How `spell` stands to `h` as two helpful spells of one family and the
/// same family flags (mangos-classic's `SpellStacker` "multirank", which
/// forbids it for positive spells): one spell, whatever their ids, so
/// Mark of the Wild and Gift of the Wild, or Shadow Protection and its
/// Prayer, never stand together. Of two with a like effect (the same aura
/// and misc value), the stronger stays: a weaker one is refused, an equal
/// or stronger takes its place (mangos's `IsSimilarExistingAuraStronger`).
/// Two helpful armor percentages of one dispel type are one too
/// (Inspiration and Ancestral Fortitude). A passive one and an active one
/// stand together, as mangos's `IsSpellStackableWithSpell` has them, and
/// so do their periodic effects
/// (two HoTs of a family's flags, as Thistle Tea's Swiftmend tests have
/// them).
fn multirank_relation_to(
    data: &crate::game::data::GameData,
    spell: &SpellTemplate,
    h: &Holder,
) -> Option<GroupRelation> {
    let old = data.spells.get(h.spell)?;
    if h.spell == spell.id
        || data.spells.same_kind(h.spell, spell.id)
        || !h.positive
        || !positive(spell)
        || old.attr(0, sd::ATTR_PASSIVE) != spell.attr(0, sd::ATTR_PASSIVE)
        || triggers_one_another(spell, Some(old))
    {
        return None;
    }
    let multirank = spell.family_name != 0
        && spell.family_flags != 0
        && old.family_name == spell.family_name
        && old.family_flags == spell.family_flags;
    // Two helpful armor percentages of one dispel type, whatever their
    // families (Inspiration and Ancestral Fortitude: mangos's
    // `SPELL_AURA_MOD_RESISTANCE_PCT` case).
    let armor_pct = spell.dispel != 0 && spell.dispel == old.dispel;
    // The effects mangos's stacker lets stand together whatever the
    // family: periodic ones (damage, healing, a trigger, mana), and the
    // helpful haste and damage done; armor of a spell that stacks as
    // damage over time does.
    const STACK_ANYWAY: &[u32] = &[3, 8, 20, 21, 23, 24, 53, 64, 89, 162, 13, 79, 96, 138, 140];
    let dot_stacking = spell.attr(3, sd::ATTR_EX3_DOT_STACKING);
    let level = h.level.max(1);
    let mut alike = false;
    let (mut theirs, mut mine) = (0i64, 0i64);
    for (i, e) in spell.effects.iter().enumerate() {
        let kept_apart =
            STACK_ANYWAY.contains(&e.aura) || (e.aura == AURA_MOD_RESISTANCE && dot_stacking);
        let unstackable =
            (multirank && !kept_apart) || (e.aura == AURA_MOD_RESISTANCE_PCT && armor_pct);
        if e.aura == 0 || !unstackable {
            continue;
        }
        for x in h.effects.iter() {
            let o = &old.effects[(x.effect as usize).min(2)];
            if o.aura == e.aura && o.misc_value == e.misc_value {
                alike = true;
                theirs += i64::from(x.amount.abs());
                mine += i64::from(crate::game::map::spells::points(spell, i, level).abs());
            }
        }
    }
    alike.then_some(if theirs > mine {
        GroupRelation::Blocked
    } else {
        GroupRelation::Replaces
    })
}

/// How `spell` stands to `h` by the vmangos spell groups they share.
fn groups_relation_to(
    data: &crate::game::data::GameData,
    spell: &SpellTemplate,
    h: &Holder,
) -> Option<GroupRelation> {
    let mine = data.spells.groups.get(&spell.id)?;
    if h.spell == spell.id || h.passive || data.spells.same_kind(h.spell, spell.id) {
        return None;
    }
    let theirs = data.spells.groups.get(&h.spell)?;
    let other = data.spells.get(h.spell);
    if other.is_some_and(|o| o.attr(0, sd::ATTR_HIDDEN)) || triggers_one_another(spell, other) {
        return None;
    }
    for g in mine.iter().filter(|g| theirs.contains(g)) {
        match data.spells.group_rules.get(g).copied().unwrap_or(0) {
            1 => return Some(GroupRelation::Replaces),
            3 => {
                let order = |id| data.spells.group_order(*g, id);
                return Some(if order(h.spell) > order(spell.id) {
                    GroupRelation::Blocked
                } else {
                    GroupRelation::Replaces
                });
            }
            _ => {}
        }
    }
    None
}

/// Whether `a` triggers `b` or `b` triggers `a` by an effect: a parent and
/// its own triggered aura stand together (Thistle Tea's "direct trigger
/// pairs preserve their parent").
fn triggers_one_another(a: &SpellTemplate, b: Option<&SpellTemplate>) -> bool {
    let Some(b) = b else {
        return false;
    };
    a.effects
        .iter()
        .any(|e| e.trigger_spell == b.id && b.id != 0)
        || b.effects
            .iter()
            .any(|e| e.trigger_spell == a.id && a.id != 0)
}

/// Whether a group's stronger spell already on `target` refuses `spell`
/// there (rule 3 keeping its highest): a helpful cast at one unit is told
/// so before it costs anything (Thistle Tea's "positive single-target
/// upgrades reject weaker casts on self and others").
pub(crate) fn group_blocks(map: &Map, target: Guid, spell: &SpellTemplate) -> bool {
    auras(map, target)
        .is_some_and(|a| group_relation(&map.realm.data, a, spell) == Some(GroupRelation::Blocked))
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
        // Never replaced: from the same item it is refreshed in place
        // (below); from another item, the two stand together, as mangos's
        // holders of one spell and caster from two items do.
        return false;
    }
    // Crowd control of a diminishing returns group takes the place of
    // another of its group, whoever cast it (mangos-classic's
    // `RemoveNoStackAurasDueToAuraHolder`: "remove any existing holders
    // from the same diminishing returns group by treating as unique"; its
    // `DIMINISHING_LIMITONLY` spells are of no group here).
    if new.dr.is_some() && old.dr == new.dr {
        return true;
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
        // Gear's passive stands beside another's cast of its spell, as
        // mangos's `AddSpellAuraHolder` lets a passive stack with itself
        // (Thistle Tea's sources apart).
        if new.passive && new.equipped {
            return false;
        }
        // Another caster's: replaced unless the effects stand together (or
        // its stacks are anyone's).
        return !spell.custom(sd::CUSTOM_ALLOW_STACK_BETWEEN_CASTERS)
            && !cross_caster_coexist(spell, old, effects);
    }
    if data.spells.same_kind(old.spell, new.spell) {
        return old.caster == new.caster || !cross_caster_coexist(spell, old, effects);
    }
    group_relation_to(data, spell, old) == Some(GroupRelation::Replaces)
}

/// The slot fields for holder `index` of `target`.
fn write_slot(map: &mut Map, target: Guid, index: usize) {
    let Some(h) = auras(map, target)
        .and_then(|a| a.holders.get(index))
        .cloned()
    else {
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

/// `caster`'s holder of `spell` (from `item`) on `target` at `stacks` (a
/// saved aura's, at login): its slot shows them and the stats count them.
pub fn set_stacks(
    map: &mut Map,
    target: Guid,
    spell: u32,
    caster: Guid,
    item: Option<Guid>,
    stacks: u32,
) {
    let Some(a) = auras_mut(map, target) else {
        return;
    };
    let Some(i) = a
        .holders
        .iter()
        .position(|h| h.spell == spell && h.caster == caster && h.item == item)
    else {
        return;
    };
    if a.holders[i].stacks == stacks {
        return;
    }
    a.holders[i].stacks = stacks;
    write_slot(map, target, i);
    recalc(map, target);
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
    f.set_u32(
        flags_word,
        (old & !(0xF << shift)) | ((flags & 0xF) << shift),
    );
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
    for h in &gone {
        crate::game::map::spell_areas::prerequisite_changed(map, target, h.spell);
    }
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

/// A channel pushed back by `cut`: `caster`'s holders of `spell` on
/// `target` end that much sooner, and so does their crowd control's break
/// (Thistle Tea's `delay_source_spell`).
pub fn delay_source(map: &mut Map, target: Guid, spell: u32, caster: Guid, cut: Duration) {
    let now = map.now;
    let Some(a) = auras_mut(map, target) else {
        return;
    };
    let mut shown = Vec::new();
    for h in a
        .holders
        .iter_mut()
        .filter(|h| h.spell == spell && h.caster == caster)
    {
        if let Some(t) = h.expires {
            let t = t.checked_sub(cut).unwrap_or(now).max(now);
            h.expires = Some(t);
            if let Some(slot) = h.slot {
                shown.push((slot, t.saturating_duration_since(now).as_millis() as u32));
            }
        }
        if let Some(b) = h.heartbeat.as_mut()
            && let Some(at) = b.break_at
        {
            b.break_at = Some(at.checked_sub(cut).unwrap_or(now).max(now));
        }
    }
    if let Some(p) = map.players.get(&target) {
        for (slot, ms) in shown {
            p.outbound.send(&SMSG_UPDATE_AURA_DURATION {
                aura_slot: slot as u8,
                aura_duration: ms,
            });
        }
    }
}

/// `target`'s auras of `spell` from `caster` end no later than `until`,
/// the client told of a slotted one's new duration (an area's, its area
/// cut short: mangos's `DelaySpellAuraHolder`).
pub fn shorten_to(map: &mut Map, target: Guid, spell: u32, caster: Guid, until: Instant) {
    let now = map.now;
    let until = until.max(now);
    let Some(a) = auras_mut(map, target) else {
        return;
    };
    let mut shown = Vec::new();
    for h in a
        .holders
        .iter_mut()
        .filter(|h| h.spell == spell && h.caster == caster)
    {
        if h.expires.is_none_or(|t| t > until) {
            h.expires = Some(until);
            if let Some(slot) = h.slot {
                shown.push((
                    slot,
                    until.saturating_duration_since(now).as_millis() as u32,
                ));
            }
        }
    }
    if let Some(p) = map.players.get(&target) {
        for (slot, ms) in shown {
            p.outbound.send(&SMSG_UPDATE_AURA_DURATION {
                aura_slot: slot as u8,
                aura_duration: ms,
            });
        }
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

/// `count` stacks of `caster`'s `spell` come off (the holder, with none
/// left).
pub fn remove_stacks(map: &mut Map, target: Guid, spell: u32, caster: Guid, count: u32) {
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
    if a.holders[i].stacks > count {
        a.holders[i].stacks -= count;
        write_slot(map, target, i);
        recalc(map, target);
    } else {
        remove_spell(map, target, spell, Some(caster));
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

/// A linked aura's spell: its auras on `target` from the same caster, for
/// as long as the linked aura has left (Thistle Tea's `Aura.Linked`).
fn linked(map: &mut Map, target: Guid, h: &Holder, spell: u32) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
        .map(|i| (i, crate::game::map::spells::points(s, i, h.level)))
        .collect();
    // Its own duration, cut short by what the linked aura has left: a child
    // shorter than its parent runs out on its own and isn't made again
    // until the parent is (Thistle Tea's `Linked`).
    let left = h
        .expires
        .map(|t| t.saturating_duration_since(map.now).as_millis() as u32);
    let own = data.spells.duration(s, h.level);
    let left = match (own, left) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let ctx = Context {
        item: h.item,
        duration: Some(left),
        triggered: true,
        linked: true,
        ..Default::default()
    };
    let _ = apply_spell(map, h.caster, target, s, &effects, ctx);
    // It goes with the aura that holds it, and what it triggers with it.
    if let Some(a) = auras_mut(map, target)
        && let Some(child) = a
            .holders
            .iter_mut()
            .find(|x| x.spell == spell && x.caster == h.caster && x.passive)
    {
        child.source = Some((h.spell, h.caster));
    }
}

/// An aura effect began: what it does at once.
/// Fear or confusion takes hold of `target`: its fight and cast stop, and
/// it is flagged fleeing or confused.
fn taken_over(map: &mut Map, target: Guid, kind: u32) {
    crate::game::map::combat::stop_attack(map, target);
    crate::game::map::spells::interrupt(map, target);
    if let Some(f) = map.unit_fields_mut(target) {
        let flag = if kind == AURA_MOD_FEAR {
            UNIT_FLAG_FLEEING
        } else {
            UNIT_FLAG_CONFUSED
        };
        f.set_flag(UNIT_FIELD_FLAGS, flag, true);
    }
}

/// What moves `target` may have changed (fear, confusion, what prevents
/// fleeing): a player's control is brought in line, a creature's walk
/// under way stops for its new mode, or for none, at once.
fn controls_changed(map: &mut Map, target: Guid) {
    crate::game::map::player_motion::control(map, target);
    if crate::game::guid::is_unit(target) {
        crate::game::map::creature_combat::control_changed(map, target);
    }
}

fn started(map: &mut Map, target: Guid, h: &Holder, a: &Aura) {
    let kind = a.kind;
    let misc = a.misc;
    match kind {
        AURA_FORCE_REACTION => crate::game::map::reputation::forced_reaction_changed(
            map,
            target,
            misc.max(0) as u32,
            a.amount,
            true,
        ),
        AURA_LINKED => linked(map, target, h, a.trigger),
        AURA_MOD_STUN | AURA_MOD_ROOT => {
            // A stun stops a player's swings. A root stops no one's, and
            // neither takes a creature out of its fight: it fights on
            // where it stands, and chases once it's free (mangos's
            // `HandleAuraModRoot` and `SetStunned` stop no attack). A
            // Young Wolf under Frost Nova went back to its wander, its
            // victim still selected.
            if kind == AURA_MOD_STUN && !crate::game::guid::is_unit(target) {
                crate::game::map::combat::stop_attack(map, target);
            }
            if kind == AURA_MOD_STUN
                && let Some(f) = map.unit_fields_mut(target)
            {
                f.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_STUNNED,
                    true,
                );
            }
            crate::game::map::movement::send_root(map, target, true);
            if kind == AURA_MOD_STUN {
                crate::game::map::spells::interrupt(map, target);
            }
            // A run the unit is driven on stops where it is.
            crate::game::map::creature_combat::immobilized(map, target);
            crate::game::map::player_motion::immobilized(map, target);
        }
        AURA_MOD_FEAR | AURA_MOD_CONFUSE => {
            // A fear under an aura that prevents fleeing (Recklessness,
            // Death Wish) holds back until that goes: no flight, the cast
            // and the fight go on (mangos's `SetFleeing`, Thistle Tea's
            // "fear applied under recklessness leaves casting intact").
            let held_back = kind == AURA_MOD_FEAR
                && auras(map, target).is_some_and(|x| x.has(AURA_PREVENTS_FLEEING));
            if !held_back {
                taken_over(map, target, kind);
            }
            // A player is moved by the server while it lasts; a creature's
            // walk under way stops for it.
            controls_changed(map, target);
        }
        // A fear under way ends while it lasts, and a creature's run (for
        // help, or a critter's escape) waits, its fleeing flag off (mangos's
        // `HandlePreventFleeing` and `SetFleeing`, Thistle Tea's critter
        // held).
        AURA_PREVENTS_FLEEING => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FLEEING, false);
            }
            if map
                .creature(target)
                .is_some_and(|c| c.engagement.flee.is_some())
            {
                crate::game::map::creature_combat::control_changed(map, target);
            }
            controls_changed(map, target);
        }
        AURA_MOD_SILENCE | AURA_MOD_PACIFY_SILENCE => {
            // Only a cast that silence prevents is cut short (mangos's
            // `HandleAuraModSilence`, Thistle Tea's `CombatControl`).
            let silenceable = map
                .caster(target)
                .and_then(|c| c.casting.as_ref())
                .and_then(|c| map.realm.data.spells.get(c.spell))
                .is_some_and(|s| {
                    s.prevention_type == crate::game::map::spells::checks::PREVENTION_SILENCE
                });
            if silenceable {
                crate::game::map::spells::interrupt(map, target);
            }
            // Both at once pacifies as well (Thistle Tea's `CombatControl`).
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SILENCED, true);
                if a.kind == AURA_MOD_PACIFY_SILENCE {
                    f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PACIFIED, true);
                }
            }
        }
        // A pacified unit's fight goes on: its blows are held while it lasts,
        // its queued strike and the extra attacks it is owed with them
        // (mangos's `HandleAuraModPacify` sets the flag alone, and
        // `AttackerStateUpdate` swings nothing under it; Thistle Tea's
        // `melee_attack_with_context`).
        AURA_MOD_PACIFY => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PACIFIED, true);
            }
        }
        AURA_MOD_SHAPESHIFT => {
            shapeshift(map, target, misc.max(0) as u8, h.spell);
        }
        AURA_TRANSFORM => {
            sync_display(map, target);
            crate::game::map::spells::interrupts::transformed(map, target, h.spell, a.effect);
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
            crate::game::map::spells::interrupts::mounted(map, target, h.spell);
        }
        AURA_TRACK_CREATURES | AURA_TRACK_RESOURCES | AURA_TRACK_STEALTHED => {
            tracking(map, target);
        }
        AURA_MOD_SCALE => sync_display(map, target),
        AURA_MOD_DISARM => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_DISARMED, true);
            }
        }
        AURA_FEIGN_DEATH => {
            // Believed or not, its Auto Shot stops, and its client is told
            // (mangos's `SetFeignDeath`, `InterruptNonMeleeSpells(true)`).
            crate::game::map::ranged::cancel(map, target, true);
            // The creatures fighting it may see through it (Thistle Tea's
            // `FeignDeath`): then it only stops attacking.
            if crate::game::map::spells::class_scripts::feign_death_resisted(map, target, h.spell) {
                crate::game::map::combat::stop_attack(map, target);
                if let Some(p) = map.players.get(&target) {
                    p.outbound.send_frame(
                        crate::game::data::items::Body::default().frame(
                            crate::game::map::spells::class_scripts::SMSG_FEIGN_DEATH_RESISTED,
                        ),
                    );
                }
            } else {
                crate::game::map::combat::leave_combat(map, target);
                crate::game::map::ai::forget(map, target);
                crate::game::map::combat::feigned(map, target);
                crate::game::map::spells::interrupts::concealed(map, target, h.spell);
            }
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FEIGN_DEATH, true);
                f.set_flag(UNIT_DYNAMIC_FLAGS, 0x20, true);
            }
        }
        AURA_MOD_TAUNT => {
            crate::game::map::ai::taunted(map, target, h.caster);
        }
        // What the unit sees through changed: what it sees is worked out
        // again now, not when it next moves (mangos's
        // `HandleInvisibilityDetect`, `UpdateVisibilityForOwner`).
        AURA_MOD_STEALTH_DETECT | AURA_MOD_INVISIBILITY_DETECTION => {
            crate::game::map::visibility::stealth_changed(map, target);
        }
        AURA_MOD_STEALTH | AURA_MOD_INVISIBILITY => {
            crate::game::map::combat::stop_attack(map, target);
            // Stealth's creep flag is of the visibility byte (3), not the
            // form's (2): mangos's `UNIT_VIS_FLAG_CREEP`.
            if kind == AURA_MOD_STEALTH
                && let Some(f) = map.unit_fields_mut(target)
            {
                f.set_byte(
                    UNIT_FIELD_BYTES_1,
                    VIS_FLAGS,
                    VIS_FLAG_CREEP | f.get_byte(UNIT_FIELD_BYTES_1, VIS_FLAGS),
                );
            }
            // A player's own client shows them stealthed, or glowing
            // invisible (mangos's `PLAYER_FIELD_BYTE2_STEALTH` and
            // `INVISIBILITY_GLOW`).
            if let Some(p) = map.players.get_mut(&target) {
                let flag = if kind == AURA_MOD_STEALTH {
                    BYTE2_STEALTH
                } else {
                    BYTE2_INVISIBILITY_GLOW
                };
                let had = p.fields.get_byte(PLAYER_FIELD_BYTES2, 1);
                p.fields.set_byte(PLAYER_FIELD_BYTES2, 1, had | flag);
            }
            crate::game::map::visibility::stealth_changed(map, target);
            crate::game::map::spells::interrupts::concealed(map, target, h.spell);
        }
        AURA_UNTRACKABLE => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_byte(
                    UNIT_FIELD_BYTES_1,
                    VIS_FLAGS,
                    VIS_FLAG_UNTRACKABLE | f.get_byte(UNIT_FIELD_BYTES_1, VIS_FLAGS),
                );
            }
        }
        AURA_WATER_WALK | AURA_FEATHER_FALL | AURA_HOVER => {
            if let Some(p) = map.players.get(&target) {
                match kind {
                    AURA_WATER_WALK => {
                        p.outbound.send(&SMSG_MOVE_WATER_WALK {
                            guid: target,
                            counter: p.outbound.order(Order::WaterWalk {
                                guid: target,
                                on: true,
                            }),
                        });
                    }
                    AURA_FEATHER_FALL => {
                        p.outbound.send(&SMSG_MOVE_FEATHER_FALL {
                            guid: target,
                            counter: p.outbound.order(Order::FeatherFall {
                                guid: target,
                                on: true,
                            }),
                        });
                    }
                    _ => {
                        p.outbound.send(&SMSG_MOVE_SET_HOVER {
                            guid: target,
                            counter: p.outbound.order(Order::Hover {
                                guid: target,
                                on: true,
                            }),
                        });
                    }
                }
            }
        }
        AURA_BIND_SIGHT => {
            crate::game::map::spells::sight::grant(map, h.caster, target);
        }
        AURA_FAR_SIGHT => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_guid(PLAYER_FARSIGHT, h.caster);
            }
        }
        AURA_MOD_POSSESS | AURA_MOD_CHARM | AURA_AOE_CHARM => {
            crate::game::map::pets::charmed(map, h.caster, target, kind == AURA_MOD_POSSESS);
        }
        AURA_MOD_POSSESS_PET => {
            crate::game::map::pets::eyes::look(map, h.caster, target);
        }
        AURA_ADD_FLAT_MODIFIER | AURA_ADD_PCT_MODIFIER => {
            crate::game::map::spells::mods::changed(
                map,
                target,
                a.mask,
                a.misc.max(0) as usize,
                kind == AURA_ADD_PCT_MODIFIER,
            );
            crate::game::map::pets::master_mods_changed(map, target, h.spell, a.mask);
            if reapplies(map, h.spell, a.misc) {
                reapply_own_passives(map, target, h.spell, a.mask);
            }
        }
        AURA_MOD_TOTAL_THREAT => crate::game::map::spells::threat_sync::fade_changed(map, target),
        AURA_MOD_UNATTACKABLE | AURA_AURAS_VISIBLE => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, unit_flag_of(kind), true);
            }
            if kind == AURA_MOD_UNATTACKABLE {
                crate::game::map::spells::interrupts::invulnerable(map, target, h.spell);
            }
        }
        // Beast Lore: the target's special information shows, its damage
        // and resistances to its caster (`views`). Hunter's Mark, Mind
        // Vision: the unit is tracked, by its caster alone (mangos's
        // `HandleAuraModStalked`, Thistle Tea's `StealthSync`). Who sees
        // it so changed, though the flag may not have: each observer is
        // told their own view again.
        AURA_EMPATHY | AURA_MOD_STALKED => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_DYNAMIC_FLAGS, dynflag_of(kind), true);
                f.touch(UNIT_DYNAMIC_FLAGS);
            }
        }
        AURA_GHOST => {}
        // A player's own protection from every school drops what
        // invulnerability ends (a flag, silithyst); one's own is helpful,
        // as Thistle Tea's holders have it.
        AURA_SCHOOL_IMMUNITY
            if (h.positive || h.caster == target)
                && map
                    .players
                    .get(&target)
                    .is_some_and(|p| p.fields.get_guid(UNIT_FIELD_CHARMEDBY).is_zero()) =>
        {
            crate::game::map::spells::interrupts::invulnerable(map, target, h.spell);
        }
        _ => {}
    }
}

/// An aura effect ended: undo what it did.
fn ended(map: &mut Map, target: Guid, h: &Holder, a: &Aura) {
    let still = |map: &Map, k: u32| auras(map, target).is_some_and(|x| x.has(k));
    match a.kind {
        AURA_FORCE_REACTION => crate::game::map::reputation::forced_reaction_changed(
            map,
            target,
            a.misc.max(0) as u32,
            a.amount,
            false,
        ),
        // The child it holds goes with it; a copy of its spell on its own
        // stays (Thistle Tea's `Linked`).
        AURA_LINKED => remove_where(map, target, |x| {
            x.spell == a.trigger && x.caster == h.caster && x.passive
        }),
        AURA_MOD_STUN | AURA_MOD_ROOT => {
            // A logout's countdown holds the player as a stun does (Thistle
            // Tea's `MovementSync` keeps the logout's restriction).
            let logging_out = map
                .players
                .get(&target)
                .is_some_and(|p| p.misc.logout_rooted);
            let stunned = still(map, AURA_MOD_STUN) || logging_out;
            let rooted = stunned || still(map, AURA_MOD_ROOT);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_STUNNED,
                    stunned,
                );
            }
            if !rooted {
                crate::game::map::movement::send_root(map, target, false);
            }
            if !rooted {
                crate::game::map::creature_combat::freed(map, target);
            }
        }
        AURA_MOD_FEAR if !still(map, AURA_MOD_FEAR) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FLEEING, false);
            }
            controls_changed(map, target);
        }
        // A creature's run held by it goes on, if it isn't over, and the fear
        // it held back takes hold (mangos's `HandlePreventFleeing` putting the
        // flight back, `SetFleeing(true)` for the first fear).
        AURA_PREVENTS_FLEEING if !still(map, AURA_PREVENTS_FLEEING) => {
            let now = map.now;
            if let Some(c) = map.creature_mut(target)
                && c.engagement.flee.is_some_and(|f| f.until > now)
            {
                c.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FLEEING, true);
                crate::game::map::creature_combat::control_changed(map, target);
            }
            if still(map, AURA_MOD_FEAR) {
                taken_over(map, target, AURA_MOD_FEAR);
                controls_changed(map, target);
            }
        }
        AURA_MOD_CONFUSE if !still(map, AURA_MOD_CONFUSE) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_CONFUSED, false);
            }
            controls_changed(map, target);
        }
        AURA_MOD_SILENCE | AURA_MOD_PACIFY_SILENCE | AURA_MOD_PACIFY => {
            // What's left of either still holds (Thistle Tea's
            // `CombatControl.sync`).
            let both = still(map, AURA_MOD_PACIFY_SILENCE);
            let silenced = both || still(map, AURA_MOD_SILENCE);
            let pacified = both || still(map, AURA_MOD_PACIFY);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SILENCED, silenced);
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PACIFIED, pacified);
            }
        }
        AURA_MOD_SHAPESHIFT if !still(map, AURA_MOD_SHAPESHIFT) => {
            shapeshift(map, target, 0, h.spell);
        }
        AURA_TRANSFORM => sync_display(map, target),
        AURA_MOUNTED if !still(map, AURA_MOUNTED) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 0);
            }
            crate::game::map::spells::interrupts::dismounted(map, target);
        }
        AURA_TRACK_CREATURES | AURA_TRACK_RESOURCES | AURA_TRACK_STEALTHED => {
            tracking(map, target);
        }
        AURA_MOD_SCALE => sync_display(map, target),
        AURA_MOD_DISARM if !still(map, AURA_MOD_DISARM) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_DISARMED, false);
            }
        }
        AURA_FEIGN_DEATH if !still(map, AURA_FEIGN_DEATH) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_DYNAMIC_FLAGS, 0x20, false);
            }
            // The body is let go unless a stun or the like holds it too.
            body_held(map, target);
            if let Some(c) = map.combat_mut(target) {
                c.feigning = false;
            }
        }
        AURA_MOD_TAUNT => {
            crate::game::map::ai::taunt_ended(map, target, h.caster);
        }
        AURA_MOD_STEALTH_DETECT | AURA_MOD_INVISIBILITY_DETECTION => {
            crate::game::map::visibility::stealth_changed(map, target);
        }
        AURA_MOD_STEALTH | AURA_MOD_INVISIBILITY => {
            if a.kind == AURA_MOD_STEALTH
                && !still(map, AURA_MOD_STEALTH)
                && let Some(f) = map.unit_fields_mut(target)
            {
                f.set_byte(
                    UNIT_FIELD_BYTES_1,
                    VIS_FLAGS,
                    f.get_byte(UNIT_FIELD_BYTES_1, VIS_FLAGS) & !VIS_FLAG_CREEP,
                );
            }
            let flag = if a.kind == AURA_MOD_STEALTH {
                BYTE2_STEALTH
            } else {
                BYTE2_INVISIBILITY_GLOW
            };
            if !still(map, a.kind)
                && let Some(p) = map.players.get_mut(&target)
            {
                let had = p.fields.get_byte(PLAYER_FIELD_BYTES2, 1);
                p.fields.set_byte(PLAYER_FIELD_BYTES2, 1, had & !flag);
            }
            crate::game::map::visibility::stealth_changed(map, target);
        }
        AURA_UNTRACKABLE if !still(map, AURA_UNTRACKABLE) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_byte(
                    UNIT_FIELD_BYTES_1,
                    VIS_FLAGS,
                    f.get_byte(UNIT_FIELD_BYTES_1, VIS_FLAGS) & !VIS_FLAG_UNTRACKABLE,
                );
            }
        }
        // A ghost goes on walking on water.
        AURA_WATER_WALK
            if !still(map, AURA_WATER_WALK) && !crate::game::map::death::is_ghost(map, target) =>
        {
            if let Some(p) = map.players.get(&target) {
                p.outbound.send(&SMSG_MOVE_LAND_WALK {
                    guid: target,
                    counter: p.outbound.order(Order::WaterWalk {
                        guid: target,
                        on: false,
                    }),
                });
            }
        }
        AURA_FEATHER_FALL if !still(map, AURA_FEATHER_FALL) => {
            if let Some(p) = map.players.get(&target) {
                p.outbound.send(&SMSG_MOVE_NORMAL_FALL {
                    guid: target,
                    counter: p.outbound.order(Order::FeatherFall {
                        guid: target,
                        on: false,
                    }),
                });
            }
        }
        AURA_HOVER if !still(map, AURA_HOVER) => {
            if let Some(p) = map.players.get(&target) {
                p.outbound.send(&SMSG_MOVE_UNSET_HOVER {
                    guid: target,
                    counter: p.outbound.order(Order::Hover {
                        guid: target,
                        on: false,
                    }),
                });
            }
        }
        AURA_BIND_SIGHT => {
            crate::game::map::spells::sight::release(map, h.caster, target);
        }
        AURA_FAR_SIGHT => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_guid(PLAYER_FARSIGHT, Guid::zero());
            }
        }
        AURA_MOD_POSSESS | AURA_MOD_CHARM | AURA_AOE_CHARM => {
            crate::game::map::pets::uncharmed(map, h.caster, target);
        }
        AURA_MOD_POSSESS_PET => {
            crate::game::map::pets::eyes::look_away(map, h.caster, target);
        }
        AURA_ADD_FLAT_MODIFIER | AURA_ADD_PCT_MODIFIER => {
            crate::game::map::spells::mods::changed(
                map,
                target,
                a.mask,
                a.misc.max(0) as usize,
                a.kind == AURA_ADD_PCT_MODIFIER,
            );
            crate::game::map::pets::master_mods_changed(map, target, h.spell, a.mask);
            if reapplies(map, h.spell, a.misc) {
                reapply_own_passives(map, target, h.spell, a.mask);
            }
        }
        AURA_MOD_TOTAL_THREAT => crate::game::map::spells::threat_sync::fade_changed(map, target),
        AURA_MOD_UNATTACKABLE | AURA_AURAS_VISIBLE if !still(map, a.kind) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_FIELD_FLAGS, unit_flag_of(a.kind), false);
            }
        }
        // The flag goes with the last of them; who sees it so changed.
        AURA_EMPATHY | AURA_MOD_STALKED => {
            let on = still(map, a.kind);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(UNIT_DYNAMIC_FLAGS, dynflag_of(a.kind), on);
                f.touch(UNIT_DYNAMIC_FLAGS);
            }
        }
        _ => {}
    }
}

/// The dynamic flag an Empathy or a mark shows.
fn dynflag_of(kind: u32) -> u32 {
    if kind == AURA_EMPATHY {
        crate::game::map::pets::DYNFLAG_SPECIALINFO
    } else {
        DYNFLAG_TRACK_UNIT
    }
}

/// A player's tracking fields from its auras (Thistle Tea's `PlayerSync`):
/// each creature and resource kind's bit (a kind outside the field's 32
/// none), and Track Hidden's flag (mangos's `PLAYER_FIELD_BYTE_TRACK_STEALTHED`).
fn tracking(map: &mut Map, target: Guid) {
    let Some(a) = map.players.get(&target).map(|p| &p.auras) else {
        return;
    };
    let bits = |kind: u32| {
        a.effects()
            .filter(|(_, x)| x.kind == kind && (1..=32).contains(&x.misc))
            .fold(0u32, |b, (_, x)| b | 1 << (x.misc - 1))
    };
    let (creatures, resources) = (bits(AURA_TRACK_CREATURES), bits(AURA_TRACK_RESOURCES));
    let hidden = a.has(AURA_TRACK_STEALTHED);
    if let Some(f) = map.unit_fields_mut(target) {
        f.set_u32(PLAYER_TRACK_CREATURES, creatures);
        f.set_u32(PLAYER_TRACK_RESOURCES, resources);
        let flags = f.get_byte(PLAYER_FIELD_BYTES, 0);
        f.set_byte(
            PLAYER_FIELD_BYTES,
            0,
            if hidden { flags | 0x02 } else { flags & !0x02 },
        );
    }
}

/// The unit flag an aura sets, as Thistle Tea's unit sync does.
fn unit_flag_of(kind: u32) -> u32 {
    match kind {
        AURA_MOD_UNATTACKABLE => 0x0001_0000,
        _ => 0x0800_0000,
    }
}

/// The unit flags vmangos's dummy auras set on a creature while it holds
/// them, as Thistle Tea's `DummyFlags` has them: Stoned leaves the statues
/// of Uldaman and Blackrock Depths unselectable.
fn dummy_flag(spell: u32) -> u32 {
    match spell {
        10255 => 0x0200_0000,
        _ => 0,
    }
}

/// A dummy aura's flag set as it comes, and cleared as the last of it goes
/// (whatever else set the flag).
fn dummy_flags(map: &mut Map, target: Guid, spell: u32, on: bool) {
    let flag = dummy_flag(spell);
    if flag == 0 || (!on && auras(map, target).is_some_and(|a| a.has_spell(spell))) {
        return;
    }
    if let Some(c) = map.creature_mut(target) {
        c.fields.set_flag(UNIT_FIELD_FLAGS, flag, on);
    }
}

/// A holder came off: what its going does.
fn removed(map: &mut Map, target: Guid, h: &Holder) {
    // The lasting auras it triggered go with it, and theirs with them.
    let source = Some((h.spell, h.caster));
    if auras(map, target).is_some_and(|a| a.holders.iter().any(|x| x.source == source)) {
        remove_where(map, target, |x| x.source == source);
    }
    if !h.positive {
        crate::game::map::spells::dr_aura_gone(map, target, h.spell);
    }
    if h.spell == crate::game::map::outdoor_pvp::SPELL_CARRYING {
        crate::game::map::outdoor_pvp::carrying_ended(map, target);
    }
    dummy_flags(map, target, h.spell, false);
    if holds_body(&map.realm.data, h) {
        body_held(map, target);
    }
    crate::game::map::captured::ended(map, h.spell, h.caster);
    // A cooldown waiting for the aura is its caster's, whomever it was on
    // (mangos's `SpellAuraHolder` removal: `caster->AddCooldown`).
    crate::game::map::spells::cooldowns::aura_gone(map, h.caster, h.spell);
    let expired = h.expires.is_some_and(|t| t <= map.now);
    crate::game::map::spells::redemption::aura_gone(map, target, h.spell, expired);
    crate::game::map::spells::class_scripts::aura_gone(map, target, h);
    crate::game::map::trinkets::removed(map, target, h);
    crate::game::map::item_scripts::aura_gone(map, target, h, expired);
    // Combo points held for a while go when the hold runs out, not when
    // it's taken off.
    if expired && h.has(AURA_RETAIN_COMBO_POINTS) {
        crate::game::map::spells::retained_combo_points_expired(map, target, h);
    }
    // A channel's aura taken off the unit it is channelled at (not run
    // out) ends the channel, the caster's own included (Evocation, a
    // bandage), as mangos's `RemoveSpellAuraHolder` and Thistle Tea's
    // channel have it.
    if !expired
        && let Some(c) = map.caster(h.caster)
        && let Some(casting) = &c.casting
        && casting.spell == h.spell
        && casting.channel_until.is_some()
        && casting.targets.unit.unwrap_or(h.caster) == target
    {
        let caster = h.caster;
        crate::game::map::spells::interrupt(map, caster);
    }
}

/// `UNIT_FIELD_BYTES_1`'s visibility byte, and its flags.
const VIS_FLAGS: u8 = 3;
const VIS_FLAG_CREEP: u8 = 0x02;
const VIS_FLAG_UNTRACKABLE: u8 = 0x04;
/// `PLAYER_FIELD_BYTES2`'s second byte: stealthed, and invisible.
const BYTE2_STEALTH: u8 = 0x20;
const BYTE2_INVISIBILITY_GLOW: u8 = 0x40;

/// `UNIT_DYNAMIC_FLAGS`'s tracked unit (`UNIT_DYNFLAG_TRACK_UNIT`).
pub const DYNFLAG_TRACK_UNIT: u32 = 0x2;

pub const UNIT_FLAG_SILENCED: u32 = 0x0000_2000;
pub const UNIT_FLAG_PACIFIED: u32 = 0x0002_0000;
pub const UNIT_FLAG_DISARMED: u32 = 0x0020_0000;
pub const UNIT_FLAG_CONFUSED: u32 = 0x0040_0000;
pub const UNIT_FLAG_FLEEING: u32 = 0x0080_0000;
/// The body is held: no emote and no change of posture (mangos's
/// `UNIT_FLAG_PREVENT_ANIM`), while an aura of a spell with
/// `SPELL_ATTR_EX_PREVENTS_ANIM` or a feign lasts.
pub const UNIT_FLAG_PREVENT_ANIM: u32 = 0x2000_0000;
/// Feign Death's flag is the same.
pub const UNIT_FLAG_FEIGN_DEATH: u32 = UNIT_FLAG_PREVENT_ANIM;

/// Whether `h` holds its bearer's body: its spell has
/// `SPELL_ATTR_EX_PREVENTS_ANIM`, or it is a feign.
fn holds_body(data: &crate::game::data::GameData, h: &Holder) -> bool {
    h.has(AURA_FEIGN_DEATH)
        || data
            .spells
            .get(h.spell)
            .is_some_and(|s| s.attr(1, sd::ATTR_EX_PREVENTS_ANIM))
}

/// `UNIT_FLAG_PREVENT_ANIM` set while any of `target`'s auras holds its
/// body, and cleared with the last (mangos's `_AddSpellAuraHolder` and
/// `_RemoveSpellAuraHolder`).
fn body_held(map: &mut Map, target: Guid) {
    let data = map.realm.data.clone();
    let held = auras(map, target).is_some_and(|a| a.holders.iter().any(|h| holds_body(&data, h)));
    if let Some(f) = map.unit_fields_mut(target) {
        f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PREVENT_ANIM, held);
    }
}

/// Shapeshift forms.
pub const FORM_CAT: u8 = 1;
pub const FORM_TREE: u8 = 2;
pub const FORM_TRAVEL: u8 = 3;
pub const FORM_AQUA: u8 = 4;
pub const FORM_BEAR: u8 = 5;
pub const FORM_GHOUL: u8 = 7;
pub const FORM_DIREBEAR: u8 = 8;
pub const FORM_CREATURE_BEAR: u8 = 14;
pub const FORM_GHOST_WOLF: u8 = 16;
pub const FORM_SPIRIT_OF_REDEMPTION: u8 = 32;
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

/// A form's passive on the unit in it, its points with the unit's spell
/// modifiers to it (mangos's `CalculateSpellEffectValue`): Feral
/// Instinct's threat on Bear Form's (21178).
fn apply_form_passive(map: &mut Map, target: Guid, t: &SpellTemplate) {
    let m = crate::game::map::spells::mods::for_spell(map, target, t);
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| t.effects[*i].effect == 6 && t.effects[*i].aura != 0)
        .map(|i| (i, m.points(t, i, crate::game::map::spells::points(t, i, 1))))
        .collect();
    let _ = apply_spell(map, target, target, t, &effects, Context::default());
}

/// A unit's spell modifiers changed (`spell`'s, touching its family's
/// spells whose flags meet `mask`): the permanent passives of its own they
/// touch go on again, their points as the modifiers now make them
/// (mangos's `Aura::ReapplyAffectedPassiveAuras` on the unit itself): a
/// form's, or a learned spell's.
pub fn reapply_own_passives(map: &mut Map, g: Guid, spell: u32, mask: u64) {
    use crate::game::map::spells::mods::touches;
    let data = map.realm.data.clone();
    let Some(family) = data.spells.get(spell).map(|t| t.family_name) else {
        return;
    };
    let form = form(map, g);
    let affected: Vec<u32> = auras(map, g)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| h.caster == g && h.passive && h.expires.is_none() && h.spell != spell)
                .filter(|h| {
                    !h.effects
                        .iter()
                        .any(|x| matches!(x.kind, AURA_ADD_FLAT_MODIFIER | AURA_ADD_PCT_MODIFIER))
                })
                .filter(|h| {
                    data.spells
                        .get(h.spell)
                        .is_some_and(|t| t.passive() && touches(family, mask, t))
                })
                .map(|h| h.spell)
                .collect()
        })
        .unwrap_or_default();
    for s in affected {
        let Some(t) = data.spells.get(s) else {
            continue;
        };
        remove_spell(map, g, s, Some(g));
        if form_passives(form).contains(&s) {
            apply_form_passive(map, g, t);
        } else {
            crate::game::map::spells::apply_passive(map, g, s);
        }
    }
}

/// Whether a modifier of `spell`'s, of operation `op`, puts its bearer's
/// own passives on again as it comes or goes: not one with charges, nor
/// one of the times, costs, speeds or attack power (mangos's
/// `Aura::ReapplyAffectedPassiveAuras`).
fn reapplies(map: &Map, spell: u32, op: i32) -> bool {
    use crate::game::map::spells::mods::*;
    let charged = map
        .realm
        .data
        .spells
        .get(spell)
        .is_some_and(|t| t.proc_charges > 0);
    let skipped = [
        OP_DURATION,
        OP_CHARGES,
        OP_NOT_LOSE_CASTING_TIME,
        OP_CASTING_TIME,
        OP_COOLDOWN,
        OP_COST,
        OP_ACTIVATION_TIME,
        OP_GLOBAL_COOLDOWN,
        OP_SPEED,
        OP_HASTE,
        OP_ATTACK_POWER,
    ];
    !charged && !skipped.contains(&(op.max(0) as usize))
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
    sync_display(map, target);
    let data = map.realm.data.clone();
    // A form left ends the unit's own auras that hang on forms (Thistle
    // Tea's `Shapeshift.interrupt_holders`); the new form's come back
    // below.
    if before != 0 && form != before {
        remove_where(map, target, |h| {
            h.caster == target
                && h.spell != by
                && data
                    .spells
                    .get(h.spell)
                    .is_some_and(crate::game::map::spells::ends_with_form)
        });
    }
    for s in form_passives(before) {
        remove_spell(map, target, *s, Some(target));
    }
    for s in form_passives(form) {
        if let Some(t) = data.spells.get(*s) {
            apply_form_passive(map, target, t);
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
    // A warrior's stance keeps only the rage Tactical Mastery holds on to
    // (Thistle Tea's `retained_stance_rage`: 5 a rank, override scripts
    // 831 to 835).
    if form != 0 && form != before && map.players.get(&target).is_some_and(|p| p.row.class == 1) {
        let kept = auras(map, target).map_or(0, |a| {
            a.effects()
                .filter(|(_, x)| {
                    x.kind == AURA_OVERRIDE_CLASS_SCRIPTS && (831..=835).contains(&x.misc)
                })
                .map(|(_, x)| ((x.misc - 830) * 50) as u32)
                .max()
                .unwrap_or(0)
        });
        if let Some(f) = map.unit_fields_mut(target) {
            let rage = f.get_u32(UNIT_FIELD_POWER1 + 1);
            f.set_u32(UNIT_FIELD_POWER1 + 1, rage.min(kept));
        }
    }
    if form != before {
        crate::game::map::spells::scripts::shapeshifted(map, target, before, form);
    }
    if form != before {
        // Entering a shapeshifted form (not a stance's like) breaks what
        // shapeshifting cancels, but the form.
        if form != 0 && !matches!(form, 17 | 18 | 19 | 28 | 30 | 31) {
            remove_where(map, target, |h| {
                h.spell != by && h.interrupt_flags & INTERRUPT_SHAPESHIFT != 0
            });
        }
        // Druid forms shake off roots and snares, going in and coming out.
        let cleansing = |f: u8| matches!(f, 1..=5 | 8 | 31);
        if cleansing(form) || cleansing(before) {
            remove_where(map, target, |h| {
                crate::game::map::spells::class_scripts::shapeshift_removable(&data, h)
            });
        }
        if map.players.contains_key(&target) {
            crate::game::map::spells::restore_form_passives(map, target);
            // Gear whose spells name forms: held to this one.
            crate::game::map::inventory::sync_gear(map, target);
        }
    }
}

/// The look a form gives a unit of `race`, as mangos's
/// `HandleAuraModShapeshift` has them: the Horde's own cat, bear and
/// moonkin; none for the forms that keep the unit's own.
fn form_display(form: u8, race: u8) -> u32 {
    let horde = matches!(race, 2 | 5 | 6 | 8);
    match form {
        FORM_CAT if horde => 8571,
        FORM_CAT => 892,
        FORM_TREE => 864,
        FORM_TRAVEL => 632,
        FORM_AQUA => 2428,
        FORM_BEAR | FORM_DIREBEAR if horde => 2289,
        FORM_BEAR | FORM_DIREBEAR => 2281,
        FORM_GHOUL if !horde => 10045,
        FORM_CREATURE_BEAR => 902,
        FORM_GHOST_WOLF => 4613,
        FORM_MOONKIN if horde => 15375,
        FORM_MOONKIN => 15374,
        FORM_SPIRIT_OF_REDEMPTION => 16031,
        _ => 0,
    }
}

/// The Orb of Deception: a transform of no creature, into the other side's
/// race of the wearer's own sex.
const ORB_OF_DECEPTION: u32 = 16739;

/// The Orb of Deception's look for a wearer's native model, as mangos's
/// `OrbOfDeception` script picks it.
fn deception_display(native: u32) -> Option<u32> {
    Some(match native {
        1479 => 10134,
        1478 => 10135,
        59 => 10136,
        49 => 10137,
        50 => 10138,
        51 => 10139,
        52 => 10140,
        53 => 10141,
        54 => 10142,
        55 => 10143,
        56 => 10144,
        58 => 10145,
        57 => 10146,
        60 => 10147,
        1563 => 10148,
        1564 => 10149,
        _ => return None,
    })
}

/// The look a unit shows: its newest transform's (a harmful one's before
/// any helpful), else its form's, else its own, as mangos's
/// `RestoreDisplayId` and Thistle Tea's `Appearance` work it out whenever
/// a transform or a form comes or goes.
fn sync_display(map: &mut Map, target: Guid) {
    let Some((transform, growth)) = shown(map, target) else {
        return;
    };
    let Some(f) = map.unit_fields_mut(target) else {
        return;
    };
    let form = form_display(
        f.get_byte(UNIT_FIELD_BYTES_1, 2),
        f.get_byte(UNIT_FIELD_BYTES_0, 0),
    );
    let display = transform
        .map(|(d, _)| d)
        .or((form != 0).then_some(form))
        .unwrap_or_else(|| f.get_u32(UNIT_FIELD_NATIVEDISPLAYID));
    if display != 0 && f.get_u32(UNIT_FIELD_DISPLAYID) != display {
        f.set_u32(UNIT_FIELD_DISPLAYID, display);
    }
    fit_size(map, target, transform, growth);
    fit_weapons(map, target, transform.and_then(|(_, entry)| entry));
}

/// A unit's own size, bounding radius and combat reach are now `own` (its
/// entry changed, or an event's model put on): its fields show them
/// through its transform and growths, and they are what comes back as
/// those go (mangos's `RestoreDisplayId` from the creature's current
/// template's scale), not the size it had before.
pub fn resized(map: &mut Map, target: Guid, own: [f32; 3]) {
    if let Some(f) = map.unit_fields_mut(target) {
        f.set_f32(OBJECT_FIELD_SCALE_X, own[0]);
        f.set_f32(UNIT_FIELD_BOUNDINGRADIUS, own[1]);
        f.set_f32(UNIT_FIELD_COMBATREACH, own[2]);
    }
    if let Some(a) = auras_mut(map, target) {
        a.native_size = None;
    }
    if let Some((transform, growth)) = shown(map, target) {
        fit_size(map, target, transform, growth);
    }
}

/// A transform's display and the creature it is of (none for the Orb of
/// Deception's).
type Transform = (u32, Option<u32>);

/// What a unit's auras make it show: its transform (`sync_display`'s
/// choice), and its growth, if it has one.
fn shown(map: &Map, target: Guid) -> Option<(Option<Transform>, Option<f32>)> {
    let data = &map.realm.data;
    let native = map
        .unit_fields(target)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_NATIVEDISPLAYID));
    let a = auras(map, target)?;
    let mut transforms: Vec<(Option<Instant>, bool, u32, Option<u32>)> = a
        .holders
        .iter()
        .flat_map(|h| {
            h.effects
                .iter()
                .filter(|x| x.kind == AURA_TRANSFORM)
                .map(move |x| (h.spell, h.applied, h.positive, x.misc))
        })
        .filter_map(|(spell, applied, positive, misc)| {
            if misc == 0 && spell == ORB_OF_DECEPTION {
                return deception_display(native).map(|d| (applied, positive, d, None));
            }
            let entry = misc.max(0) as u32;
            data.creature_template(entry)
                .and_then(|t| t.display_ids.iter().copied().find(|d| *d != 0))
                .map(|d| (applied, positive, d, Some(entry)))
        })
        .collect();
    // The newest put on (or refreshed) shows, a harmful one over any
    // helpful (Thistle Tea's `Appearance`, mangos preferring the negative);
    // those put on at once in their order.
    transforms.sort_by_key(|(applied, ..)| *applied);
    let transform = transforms
        .iter()
        .rev()
        .find(|(_, positive, ..)| !positive)
        .or(transforms.last())
        .map(|(_, _, d, entry)| (*d, *entry));
    let growth = a.has(AURA_MOD_SCALE).then(|| a.product(AURA_MOD_SCALE));
    Some((transform, growth))
}

/// A unit's size, bounding radius and combat reach for what it shows: its
/// transform's creature's as that creature has them (`Creature::fit_model`),
/// else its own, each grown by its growths (`SPELL_AURA_MOD_SCALE`), as
/// mangos's `RestoreDisplayId` and `UpdateModelData` and Thistle Tea's
/// `Appearance.project` derive them, never from what they were (no
/// compounding). Its own come back as they were once neither is on.
fn fit_size(map: &mut Map, target: Guid, transform: Option<Transform>, growth: Option<f32>) {
    let data = map.realm.data.clone();
    let Some(f) = map.unit_fields(target) else {
        return;
    };
    let current = [
        f.get_f32(OBJECT_FIELD_SCALE_X),
        f.get_f32(UNIT_FIELD_BOUNDINGRADIUS),
        f.get_f32(UNIT_FIELD_COMBATREACH),
    ];
    let Some(a) = auras_mut(map, target) else {
        return;
    };
    if transform.is_none() && growth.is_none() {
        let Some(own) = a.native_size.take() else {
            return;
        };
        if let Some(f) = map.unit_fields_mut(target) {
            f.set_f32(OBJECT_FIELD_SCALE_X, own[0]);
            f.set_f32(UNIT_FIELD_BOUNDINGRADIUS, own[1]);
            f.set_f32(UNIT_FIELD_COMBATREACH, own[2]);
        }
        return;
    }
    let own = *a.native_size.get_or_insert(current);
    let base = transform
        .map(|(display, entry)| {
            let template = entry.and_then(|e| data.creature_template(e));
            let chosen = template
                .and_then(|t| {
                    let i = t.display_ids.iter().position(|x| *x == display)?;
                    Some(t.display_scales[i])
                })
                .filter(|s| *s > 0.0);
            match data.display_info.get(&display) {
                Some(d) => {
                    let native = if d.scale > 0.0 { d.scale } else { 1.0 };
                    let scale = chosen.unwrap_or(native);
                    let grow = scale / native;
                    let radius = if d.bounding_radius > 0.0 {
                        d.bounding_radius
                    } else {
                        0.5
                    };
                    let reach = if d.combat_reach > 0.0 {
                        d.combat_reach
                    } else {
                        1.5
                    };
                    [scale, radius * grow, reach * grow]
                }
                None => {
                    let scale = chosen.unwrap_or(1.0);
                    let grow = scale / own[0].max(0.01);
                    [scale, own[1] * grow, own[2] * grow]
                }
            }
        })
        .unwrap_or(own);
    let growth = growth.unwrap_or(1.0).clamp(0.01, 100.0);
    if let Some(f) = map.unit_fields_mut(target) {
        f.set_f32(OBJECT_FIELD_SCALE_X, base[0] * growth);
        f.set_f32(UNIT_FIELD_BOUNDINGRADIUS, base[1] * growth);
        f.set_f32(UNIT_FIELD_COMBATREACH, base[2] * growth);
    }
}

/// A creature's weapons for its transform: the transform's creature's,
/// where it has some (mangos's `HandleAuraTransform` loading its
/// equipment), else its own, back as they were (Thistle Tea's
/// `Appearance.reconcile_equipment`).
fn fit_weapons(map: &mut Map, target: Guid, entry: Option<u32>) {
    let data = map.realm.data.clone();
    let items = entry
        .and_then(|e| data.creature_template(e))
        .and_then(|t| data.creature_equipment.get(&t.equipment_id))
        .copied();
    let Some(c) = map.creature_mut(target) else {
        return;
    };
    let fields = |c: &crate::game::map::creature::Creature| -> [u32; 9] {
        std::array::from_fn(|i| c.fields.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i as u16))
    };
    match items {
        Some(items) => {
            if c.auras.native_weapons.is_none() {
                c.auras.native_weapons = Some(fields(c));
            }
            c.set_equipment_slots(&data, items.map(Some));
        }
        None => {
            if let Some(own) = c.auras.native_weapons.take() {
                for (i, v) in (0u16..).zip(own) {
                    c.fields.set_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i, v);
                }
            }
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
    let before = crate::game::map::spells::health_sync::state(map, target);
    crate::game::map::derive::unit(map, target);
    if let Some(before) = before {
        crate::game::map::spells::health_sync::sync(map, target, before);
    }
    power_costs(map, target);
}

/// The client's cost fields by school: the flat and the share auras put
/// on spells' cost (Thistle Tea's `PowerCostSync`).
fn power_costs(map: &mut Map, target: Guid) {
    let Some(a) = auras(map, target) else {
        return;
    };
    let costs: [(i32, f32); 7] = std::array::from_fn(|school| {
        (
            a.sum_mask(AURA_MOD_POWER_COST_SCHOOL, 1 << school),
            a.sum_mask(AURA_MOD_POWER_COST_SCHOOL_PCT, 1 << school) as f32 / 100.0,
        )
    });
    if let Some(f) = map.unit_fields_mut(target) {
        for (school, (flat, share)) in (0u16..).zip(costs) {
            f.set_u32(UNIT_FIELD_POWER_COST_MODIFIER + school, flat as u32);
            f.set_f32(UNIT_FIELD_POWER_COST_MULTIPLIER + school, share);
        }
    }
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
    units.extend(map.with_auras_where(|c| !c.auras.is_empty(), |_, c| c.guid));
    let z = crate::zone!("units with auras");
    z.value(units.len() as u64);
    for g in units {
        let Some(a) = auras(map, g) else {
            continue;
        };
        let expired = a
            .holders
            .iter()
            .any(|h| h.expires.is_some_and(|t| t <= now));
        let ticks = due_ticks(a, now);
        // Nothing ticks or ends, so nothing changes before the crowd
        // control's look, and that would find nothing due either.
        if ticks.is_empty() && !expired && !crate::game::map::spells::heartbeat::any_due(a, now) {
            continue;
        }
        // Crowd control that breaks now breaks before its ticks due now
        // (Thistle Tea's `Aura.Heartbeat` runs first).
        crate::game::map::spells::heartbeat::update(map, g);
        let ticks: Vec<(Holder, Aura)> = match auras(map, g) {
            Some(a) => ticks
                .into_iter()
                .filter(|(h, x)| {
                    a.holders
                        .iter()
                        .any(|x| x.spell == h.spell && x.caster == h.caster && x.item == h.item)
                        && crate::game::map::pets::pulses(map, g, x.next_tick)
                })
                .collect(),
            None => Vec::new(),
        };
        // One tick an update, each on its aura's own cadence: an update
        // late by more than an interval leaves the next tick due at once,
        // for the next update (mangos's `Aura::Update`).
        run_ticks(map, g, ticks, now);
        if expired {
            remove_where(map, g, |h| h.expires.is_some_and(|t| t <= now));
        }
    }
}

/// The ticks due on a unit's auras by `now`.
fn due_ticks(a: &Auras, now: Instant) -> Vec<(Holder, Aura)> {
    a.effects()
        .filter(|(_, x)| x.next_tick.is_some_and(|t| t <= now))
        .map(|(h, x)| (h.clone(), x.clone()))
        .collect()
}

/// Run `ticks` on `g`, each moving its aura's next tick on by its interval.
fn run_ticks(map: &mut Map, g: Guid, ticks: Vec<(Holder, Aura)>, now: Instant) {
    for (h, t) in ticks {
        if let Some(a) = auras_mut(map, g)
            && let Some(x) = a
                .holders
                .iter_mut()
                .find(|x| x.spell == h.spell && x.caster == h.caster && x.item == h.item)
                .and_then(|x| x.effects.iter_mut().find(|e| e.effect == t.effect))
            && let Some(p) = x.period
        {
            // On the aura's own cadence, an update late or not, so a
            // late one doesn't lose the last tick (mangos's
            // `m_periodicTimer += periodictime`).
            x.next_tick = Some(x.next_tick.unwrap_or(now) + p);
            x.ticks += 1;
        }
        tick(map, g, &h, &t);
    }
}

/// A periodic aura's tick, in the combat log: `SMSG_PERIODICAURALOG` to
/// those who see its target.
fn periodic_log(map: &mut Map, h: &Holder, target: Guid, aura_type: AuraLog_AuraType) {
    if let Some(frame) = crate::game::conn::encode(&SMSG_PERIODICAURALOG {
        target,
        caster: h.caster,
        spell: h.spell,
        auras: vec![AuraLog { aura_type }],
    }) {
        map.send_to_observers(target, &frame, true);
    }
}

/// Run the ticks of `spell` from `caster` on `target` that are due now, on
/// their cadence as an update runs them (so that the update doesn't run
/// them again): another spell's last tick that a channel's end would
/// otherwise come before.
pub fn run_due(map: &mut Map, target: Guid, spell: u32, caster: Guid) {
    let now = map.now;
    let due: Vec<(Holder, Aura)> = auras(map, target)
        .map(|a| {
            due_ticks(a, now)
                .into_iter()
                .filter(|(h, _)| h.spell == spell && h.caster == caster)
                .collect()
        })
        .unwrap_or_default();
    run_ticks(map, target, due, now);
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
    // A captured animal's look, the player dead or not.
    if crate::game::map::captured::tick(map, target, h.spell, h.caster) {
        return;
    }
    let alive = map.unit_view(target).is_some_and(|u| u.alive);
    if !alive {
        return;
    }
    // An encounter's pulse that the data can't say.
    if a.kind == AURA_PERIODIC_TRIGGER_SPELL
        && crate::game::map::boss_mechanics::periodic_trigger(map, target, h, a)
    {
        return;
    }
    let stacks = h.stacks.max(1) as i32;
    match a.kind {
        // A drain of life or a funnel of it wants its caster alive here, or
        // the tick does nothing (mangos's `PeriodicTick`, Thistle Tea's
        // aura contexts); a mana drain's is below.
        AURA_PERIODIC_LEECH | AURA_PERIODIC_HEALTH_FUNNEL
            if !map.unit_view(h.caster).is_some_and(|u| u.alive) => {}
        AURA_PERIODIC_DAMAGE
        | AURA_PERIODIC_LEECH
        | AURA_PERIODIC_DAMAGE_PERCENT
        | AURA_PERIODIC_MANA_LEECH
        | AURA_POWER_BURN_MANA
            // Its auras' or a creature's template's (mangos's
            // `IsImmuneToDamage` at each tick).
            if crate::game::map::combat::school_immune(map, target, h.school)
                && !map
                    .realm
                    .data
                    .spells
                    .get(h.spell)
                    .is_some_and(|s| s.attr(0, crate::game::data::spells::ATTR_NO_IMMUNITIES)) =>
        {
            // A tick on one immune to its school (Divine Shield, Ice Block).
            crate::game::map::spells::log::immune(map, h.caster, target, h.spell);
        }
        AURA_PERIODIC_DAMAGE | AURA_PERIODIC_LEECH | AURA_PERIODIC_DAMAGE_PERCENT => {
            let amount = if a.kind == AURA_PERIODIC_DAMAGE_PERCENT {
                let max = map
                    .unit_fields(target)
                    .map(|f| f.get_u32(UNIT_FIELD_MAXHEALTH))
                    .unwrap_or(0);
                (max as i32 * a.amount * stacks / 100).max(0) as f32
            } else {
                // Curse of Agony and Starshards grow as they go.
                let ramp = if a.kind == AURA_PERIODIC_DAMAGE {
                    crate::game::map::spells::class_scripts::ramp(
                        &map.realm.data,
                        h.spell,
                        a.ticks + 1,
                    )
                } else {
                    0.0
                };
                // Ignite's amount is its crits' shares summed already
                // (Thistle Tea's `base_amount` for it).
                let stacks = if h.spell == crate::game::map::proc_scripts::IGNITE_BURN {
                    1
                } else {
                    stacks
                };
                ((a.amount * stacks).max(0) as f32 + ramp).max(0.0)
            };
            // A ramp's fraction stays until the target's modifiers are
            // on, then rounds without bias (Thistle Tea's `PeriodicDamage`).
            let amount = if h.spell == crate::game::map::item_scripts::DEATH_RAY_PERIODIC {
                crate::game::map::item_scripts::periodic_damage(
                    map,
                    target,
                    h,
                    amount.round() as u32,
                ) as f32
            } else {
                amount
            };
            crate::game::map::spells::periodic_damage_of(
                map,
                h,
                target,
                amount,
                a.kind == AURA_PERIODIC_LEECH,
            );
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
            // A heal's tick takes the healing taken of the moment (Thistle
            // Tea's `Periodic.healing_amount`); food's regeneration doesn't.
            let amount = match map.realm.data.spells.get(h.spell) {
                Some(s) if a.kind != AURA_MOD_REGEN => {
                    crate::game::map::spells::outcome::spell_healing_taken(
                        map,
                        target,
                        amount as f32,
                        s,
                        usize::from(a.effect),
                        crate::game::map::spells::outcome::Kind::Dot,
                        stacks as u32,
                        1.0,
                    ) as u32
                }
                _ => amount,
            };
            crate::game::map::spells::heal(map, h.caster, target, h.spell, amount, true);
            // A heal's tick procs what goes off on a periodic heal (Improved
            // Mend Pet, the Dreamwalker's Rejuvenation), as mangos's
            // `PeriodicTick` and Thistle Tea's periodic heal raise it.
            if a.kind == AURA_PERIODIC_HEAL {
                crate::game::map::procs::trigger(
                    map,
                    crate::game::map::procs::Event {
                        actor: h.caster,
                        target,
                        actor_flags: crate::game::map::procs::PROC_DONE_PERIODIC,
                        target_flags: crate::game::map::procs::PROC_TAKEN_PERIODIC,
                        ex: crate::game::map::procs::EX_NORMAL_HIT
                            | crate::game::map::procs::EX_PERIODIC_POSITIVE,
                        spell: Some(h.spell),
                        damage: 0,
                        school: h.school,
                        offhand: false,
                        ranged: false,
                        extra_attack: false,
                        origin: Default::default(),
                    },
                );
            }
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
                // Its multiplier, one if the data has none, with the
                // caster's modifiers to it: none of it if they take it all
                // (mangos-classic's `SPELLMOD_MULTIPLE_VALUE` in
                // `PeriodicTick`; Thistle Tea's "a zero transfer modifier
                // does not revert to the default ratio").
                let multiplier = map.realm.data.spells.get(h.spell).map_or(1.0, |s| {
                    let v = s.effects[a.effect as usize].multiple_value;
                    let v = if v > 0.0 { v } else { 1.0 };
                    crate::game::map::spells::mods::for_spell(map, h.caster, s)
                        .apply(crate::game::map::spells::mods::OP_MULTIPLE_VALUE, v)
                        .max(0.0)
                });
                let gain = (cost as f32 * multiplier) as u32;
                crate::game::map::spells::heal(map, h.caster, target, h.spell, gain, true);
            }
        }
        // A regeneration's tick gives only rage, to one alive whose power it
        // is, in combat: three fifths of its amount a 3 s tick, its points
        // being five seconds' (Anger Management's 15, a rage's 0.9; mangos's
        // `PeriodicTick`). Mana's is the regeneration's (`stats::regen`).
        AURA_MOD_POWER_REGEN => {
            let rage = a.misc == 1
                && crate::game::map::spells::shown_power(map, target) == 1
                && map.unit_view(target).is_some_and(|u| u.alive)
                && map.unit_fields(target).is_some_and(|f| {
                    f.has_flag(
                        UNIT_FIELD_FLAGS,
                        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                    )
                });
            if let Some(f) = map.unit_fields_mut(target).filter(|_| rage) {
                let add = ((a.amount * stacks).max(0) * 3 / 5) as u32;
                let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + 1);
                let now = f.get_u32(UNIT_FIELD_POWER1 + 1);
                f.set_u32(UNIT_FIELD_POWER1 + 1, now.saturating_add(add).min(max));
            }
        }
        AURA_PERIODIC_ENERGIZE | AURA_OBS_MOD_MANA => {
            // A percentage of mana is of mana, and does nothing for one with
            // none (mangos's `PeriodicTick`, Thistle Tea's
            // `restore_percent_mana`).
            let percent = a.kind == AURA_OBS_MOD_MANA;
            let power = if percent {
                0
            } else {
                a.misc.clamp(0, 4) as u16
            };
            let Some(f) = map.unit_fields_mut(target) else {
                return;
            };
            let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
            if percent && max == 0 {
                return;
            }
            let add = if percent {
                max * (a.amount * stacks).max(0) as u32 / 100
            } else {
                (a.amount * stacks).max(0) as u32
            };
            let now = f.get_u32(UNIT_FIELD_POWER1 + power);
            let after = now.saturating_add(add).min(max);
            f.set_u32(UNIT_FIELD_POWER1 + power, after);
            // Logged as its own aura with its power, as mangos's
            // SendPeriodicAuraLog and Thistle Tea log a tick; a regeneration
            // isn't.
            let logged = match a.kind {
                AURA_PERIODIC_ENERGIZE => Some(AuraLog_AuraType::PeriodicEnergize {
                    damage3: add,
                    misc_value1: u32::from(power),
                }),
                AURA_OBS_MOD_MANA => Some(AuraLog_AuraType::ObsModMana {
                    damage3: add,
                    misc_value1: u32::from(power),
                }),
                _ => None,
            };
            if let Some(aura_type) = logged {
                periodic_log(map, h, target, aura_type);
            }
            // Half of what it gave is its caster's threat on those fighting
            // the target, as a heal's is (mangos's `PeriodicTick`:
            // `threatAssist` of the gain): a percentage of mana's, and an
            // energize's of any power (Bloodrage's rage, a mana tick).
            // Thistle Tea's energize of mana makes none.
            if percent || a.kind == AURA_PERIODIC_ENERGIZE {
                crate::game::map::combat::heal_threat(
                    map,
                    h.caster,
                    target,
                    h.spell,
                    after.saturating_sub(now),
                );
            }
        }
        // A power that is one, the pool the target shows (a druid shifted
        // out of it keeps it till they shift back), and for a leech, a
        // caster alive to take it (mangos's `PeriodicTick`).
        AURA_PERIODIC_MANA_LEECH | AURA_POWER_BURN_MANA
            if !(0..5).contains(&a.misc)
                || crate::game::map::spells::shown_power(map, target) != a.misc as u16
                || (a.kind == AURA_PERIODIC_MANA_LEECH
                    && !map.unit_view(h.caster).is_some_and(|u| u.alive)) => {}
        AURA_PERIODIC_MANA_LEECH | AURA_POWER_BURN_MANA => {
            let take = (a.amount * stacks).max(0) as u32;
            let power = a.misc as u16;
            let taken = map
                .unit_fields_mut(target)
                .map(|f| {
                    let have = f.get_u32(UNIT_FIELD_POWER1 + power);
                    let t = take.min(have);
                    f.set_u32(UNIT_FIELD_POWER1 + power, have - t);
                    t
                })
                .unwrap_or(0);
            // The multiplier, with the caster's modifiers as they are now;
            // nothing back to a caster without the pool.
            let data = map.realm.data.clone();
            let mult = data.spells.get(h.spell).map_or(1.0, |s| {
                crate::game::map::spells::mods::for_spell(map, h.caster, s).apply(
                    crate::game::map::spells::mods::OP_MULTIPLE_VALUE,
                    s.effects[a.effect as usize].multiple_value,
                )
            });
            let pool = map
                .unit_fields(h.caster)
                .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXPOWER1 + power));
            let mult = if a.kind == AURA_PERIODIC_MANA_LEECH && pool == 0 {
                0.0
            } else {
                mult.max(0.0)
            };
            if a.kind == AURA_PERIODIC_MANA_LEECH {
                periodic_log(
                    map,
                    h,
                    target,
                    AuraLog_AuraType::PeriodicManaLeech {
                        damage: taken,
                        gain_multiplier: mult,
                        misc_value2: u32::from(power),
                    },
                );
                let gain = (taken as f32 * mult) as u32;
                let gained = map.unit_fields_mut(h.caster).map_or(0, |f| {
                    let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
                    let now = f.get_u32(UNIT_FIELD_POWER1 + power);
                    let to = (now + gain).min(max);
                    f.set_u32(UNIT_FIELD_POWER1 + power, to);
                    to - now
                });
                // Half what came back is threat on the one drained.
                if gained > 0 {
                    crate::game::map::ai::add_threat(map, target, h.caster, gained as f32 * 0.5);
                }
                crate::game::map::proc_scripts::leeched(map, target, h, power);
            } else {
                let damage = (taken as f32 * mult) as u32;
                crate::game::map::spells::periodic_damage(map, h, target, damage, false);
            }
        }
        // Frenzied Regeneration's tick, which names no spell (round 12e).
        AURA_PERIODIC_TRIGGER_SPELL
            if a.trigger == 0 && crate::game::map::spells::frenzied::is(h.spell) =>
        {
            crate::game::map::spells::frenzied::tick(map, target, a.amount);
        }
        AURA_PERIODIC_TRIGGER_SPELL
            if a.trigger != 0
                && !crate::game::map::spells::class_scripts::periodic_trigger_allowed(
                    map, target, h,
                ) => {}
        AURA_PERIODIC_TRIGGER_SPELL if a.trigger != 0 => {
            // A spell aimed at a unit the client picks goes at the unit the
            // aura is on, or a channel's at the channel's target, cast by
            // the aura's caster. Anything else (an area, a group, a place,
            // the caster, a friend) is cast by the unit the aura is on,
            // about itself, as mangos's `Aura::TriggerSpell` and
            // `PickTargetsForSpellTrigger` have it (`triggerCaster` the
            // aura's target but for `TARGET_UNIT_ENEMY` and `TARGET_UNIT`):
            // Hellfire's flames are about the warlock, not the warlock's
            // target, and Plague's about the one it is on, and a self heal
            // an enemy's aura triggers heals its bearer. An object's aura's
            // trigger is its bearer's whatever its target, an object being
            // no unit to cast it (mangos's `GetCaster`; Thistle Tea's "the
            // bearer of an aura from an object resolves its object targets
            // itself").
            use crate::game::map::spells::targets::{Implicit, implicit};
            let picked = map.realm.data.spells.get(a.trigger).is_some_and(|t| {
                matches!(
                    implicit(t.effects[0].target_a),
                    Implicit::Enemy | Implicit::AnyUnit
                )
            });
            let caster_is_unit = crate::game::guid::is_unit(h.caster)
                || crate::game::guid::is_player(h.caster);
            let by = if picked && caster_is_unit {
                h.caster
            } else {
                target
            };
            // A reagent-less aura's trigger pays its own reagents, and
            // without them doesn't go.
            let data = map.realm.data.clone();
            if let Some(spell) = data.spells.get(h.spell)
                && !crate::game::map::spells::trigger_reagents(map, by, spell, a.trigger)
            {
                return;
            }
            crate::game::map::spells::class_scripts::periodic_triggered(map, target, h);
            if !picked {
                let targets = crate::game::map::spells::Targets {
                    unit: Some(target),
                    dest: map.position_of(target),
                    origin: Some(target),
                    ..Default::default()
                };
                crate::game::map::spells::cast_by_aura_with(map, by, a.trigger, targets);
                return;
            }
            let aim = if by == target {
                map.caster(by)
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
            // A spell at an enemy with no enemy to go at goes nowhere, not
            // at the one the aura is on (Thistle Tea's
            // `redirect_trigger_target`).
            let at_enemy = map
                .realm
                .data
                .spells
                .get(a.trigger)
                .is_some_and(|t| implicit(t.effects[0].target_a) == Implicit::Enemy);
            if at_enemy && aim == by {
                return;
            }
            crate::game::map::spells::cast_by_aura(map, by, a.trigger, Some(aim));
        }
        _ => {}
    }
}

/// [`absorb`], then the shares of what is left that others take (Blessing
/// of Sacrifice, Soul Link, a Voidwalker's Sacrifice): (what's left, what
/// was absorbed or shared).
pub fn absorb_and_share(
    map: &mut Map,
    attacker: Guid,
    target: Guid,
    damage: u32,
    school: u32,
) -> (u32, u32) {
    let (left, absorbed) = absorb(map, target, damage, school);
    let after = share(map, attacker, target, left, school);
    (after, absorbed + (left - after))
}

/// The shares of `damage` from `attacker` that the casters of `target`'s
/// split auras take instead, as mangos's `CalculateDamageAbsorbAndResist`
/// and Thistle Tea's `DamageSharing` have them: none of one's own damage;
/// each from a caster alive here, of the damage's school; the flat ones
/// first, each as much as it says (its stacks rolled in), then each share
/// of what is left, rounded down. A flat share is soaked by its taker's
/// shields, a share by none; one immune to the school, or on a flight,
/// takes it all absorbed. Each is logged under its aura's spell, and the
/// attacker gets the credit. What is left.
pub fn share(map: &mut Map, attacker: Guid, target: Guid, damage: u32, school: u32) -> u32 {
    if damage == 0 || attacker == target {
        return damage;
    }
    let Some(a) = auras(map, target) else {
        return damage;
    };
    let mut links: Vec<(bool, Guid, u32, i32)> = a
        .effects()
        .filter(|(h, x)| {
            matches!(x.kind, AURA_SPLIT_DAMAGE_PCT | AURA_SPLIT_DAMAGE_FLAT)
                && h.caster != target
                && (x.misc as u32) & school != 0
        })
        .map(|(h, x)| {
            (
                x.kind == AURA_SPLIT_DAMAGE_PCT,
                h.caster,
                h.spell,
                x.amount * h.stacks.max(1) as i32,
            )
        })
        .collect();
    if links.is_empty() {
        return damage;
    }
    links.sort_by_key(|l| l.0);
    let mut left = damage;
    for (percent, to, spell, amount) in links {
        if left == 0 {
            break;
        }
        if map.unit_view(to).is_none_or(|u| !u.alive) {
            continue;
        }
        let taken = if percent {
            (u64::from(left) * amount.max(0) as u64 / 100).min(u64::from(left)) as u32
        } else {
            (amount.max(0) as u32).min(left)
        };
        if taken == 0 {
            continue;
        }
        left -= taken;
        let spared = map.players.get(&to).is_some_and(|p| p.flight.is_some())
            || auras(map, to).is_some_and(|x| x.school_immune(school));
        let (hurt, soaked) = if spared {
            (0, taken)
        } else if percent {
            (taken, 0)
        } else {
            absorb(map, to, taken, school)
        };
        let frame = crate::game::map::spells::outcome::damage_log(
            attacker,
            to,
            spell,
            school.trailing_zeros(),
            hurt,
            soaked,
            0,
            0,
            false,
        );
        map.send_to_observers(to, &frame, true);
        if hurt > 0 {
            crate::game::map::combat::deal_damage(map, attacker, to, hurt);
        }
    }
    left
}

/// Damage of `school` a unit takes, less what its absorb auras soak, then
/// its mana shields, each no more than the mana there is pays for at its
/// spell's rate (its effect's multiple value, with the unit's modifiers),
/// as mangos's `CalculateDamageAbsorbAndResist` and Thistle Tea's
/// `Absorption` have it: (what's left, what's absorbed).
pub fn absorb(map: &mut Map, target: Guid, damage: u32, school: u32) -> (u32, u32) {
    let data = map.realm.data.clone();
    let rates: Vec<(u32, f32)> = auras(map, target)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| h.has(AURA_MANA_SHIELD))
                .filter_map(|h| {
                    let s = data.spells.get(h.spell)?;
                    let e = s.effects.iter().find(|e| e.aura == AURA_MANA_SHIELD)?;
                    let m = crate::game::map::spells::mods::for_spell(map, target, s);
                    let rate = m.apply(
                        crate::game::map::spells::mods::OP_MULTIPLE_VALUE,
                        e.multiple_value,
                    );
                    Some((h.spell, rate.max(0.0)))
                })
                .collect()
        })
        .unwrap_or_default();
    let had = map
        .unit_fields(target)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_POWER1));
    let mut mana = had;
    let Some(a) = auras_mut(map, target) else {
        return (damage, 0);
    };
    let mut left = damage as i32;
    let mut used_up = Vec::new();
    for kind in [AURA_SCHOOL_ABSORB, AURA_MANA_SHIELD] {
        for h in a.holders.iter_mut() {
            for x in h.effects.iter_mut() {
                if left <= 0 {
                    break;
                }
                if x.kind != kind || (x.misc as u32) & school == 0 {
                    continue;
                }
                let mut soak = x.left.min(left).max(0);
                let rate = rates
                    .iter()
                    .find(|(spell, _)| *spell == h.spell)
                    .map_or(0.0, |r| r.1);
                if kind == AURA_MANA_SHIELD && rate > 0.0 {
                    soak = soak.min((mana as f32 / rate) as i32);
                    mana -= ((soak as f32 * rate).round() as u32).min(mana);
                }
                x.left -= soak;
                left -= soak;
                if x.left <= 0 {
                    used_up.push((h.spell, h.caster));
                }
            }
        }
    }
    if mana != had
        && let Some(f) = map.unit_fields_mut(target)
    {
        f.set_u32(UNIT_FIELD_POWER1, mana);
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
            // A fear that may not make it flee holds nothing (mangos's
            // `SetFeared` under SPELL_AURA_PREVENTS_FLEEING).
            || (a.has(AURA_MOD_FEAR) && !a.has(AURA_PREVENTS_FLEEING))
            || a.has(AURA_MOD_CONFUSE)
            || (casting && (a.has(AURA_MOD_SILENCE) || a.has(AURA_MOD_PACIFY_SILENCE)))
    })
}

/// `CMSG_CANCEL_AURA`: a player takes a helpful aura off. A stealth taken
/// off takes every stealth with it, so cancelling Stealth while Vanish's
/// own stealth is on leaves the rogue seen (Thistle Tea's `cancel_spell`
/// and its `stealth_holder?`, the user's #41; mangos's takes the spell
/// named alone).
pub fn cancel(map: &mut Map, player: Guid, spell: u32) {
    const ATTR_EX_NO_AURA_ICON: u32 = 0x1000_0000;
    let data = map.realm.data.clone();
    let s = data.spells.get(spell);
    // Not one the client shows no icon for (but tracking), not while
    // charmed, nor another's party aura (mangos's `HandleCancelAuraOpcode`).
    let hidden = s.is_some_and(|s| {
        s.attr(1, ATTR_EX_NO_AURA_ICON) && !s.effects.iter().any(|e| e.aura == AURA_TRACK_RESOURCES)
    });
    if s.is_some_and(|s| s.attr(0, sd::ATTR_CANT_CANCEL)) || hidden {
        return;
    }
    let party =
        s.is_some_and(|s| s.has_effect(crate::game::map::spells::EFFECT_APPLY_AREA_AURA_PARTY));
    let Some(stealth) = map
        .players
        .get(&player)
        .filter(|p| p.fields.get_guid(UNIT_FIELD_CHARMEDBY).is_zero())
        .and_then(|p| {
            p.auras
                .holders
                .iter()
                .find(|h| h.spell == spell && h.positive && !h.passive)
                .filter(|h| !(party && h.caster != player))
                .map(stealth_holder)
        })
    else {
        return;
    };
    remove_where(map, player, |h| {
        !h.passive && (h.spell == spell || (stealth && stealth_holder(h)))
    });
}

/// A stealth: its aura, or the form (Thistle Tea's `stealth_holder?`).
fn stealth_holder(h: &Holder) -> bool {
    h.effects.iter().any(|x| {
        x.kind == AURA_MOD_STEALTH
            || (x.kind == AURA_MOD_SHAPESHIFT && x.misc == i32::from(FORM_STEALTH))
    })
}

/// On death every aura but the passive and death-persistent ones goes, and
/// so does the cast.
pub fn clear_on_death(map: &mut Map, g: Guid) {
    // Its Hunter's Mark on another goes with it.
    crate::game::map::spells::single_target::caster_died(map, g);
    crate::game::map::spells::interrupt(map, g);
    // Death forgets its diminishing returns (mangos's `ClearDiminishings`
    // at `JUST_DIED`).
    if let Some(c) = map.caster_mut(g) {
        c.diminishing.clear();
    }
    let data = map.realm.data.clone();
    let death_casts = crate::game::map::boss_mechanics::death_casts(map, g);
    let doomed = crate::game::map::spells::script_checks::doomed(map, g);
    remove_where(map, g, |h| {
        !h.passive
            && !data
                .spells
                .get(h.spell)
                .is_some_and(|s| s.attr(3, sd::ATTR_EX3_DEATH_PERSISTENT))
    });
    crate::game::map::boss_mechanics::cast_on_death(map, g, death_casts);
    crate::game::map::spells::script_checks::doom(map, &doomed);
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
