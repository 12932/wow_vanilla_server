//! A unit's derived stats, worked out again whenever its gear, level or
//! auras change: stats, health and power, armor and resistances, attack
//! power, weapon damage, attack and cast speed, and the chances to crit,
//! hit, dodge, parry and block. The formulas are vmangos's, as Thistle Tea's
//! `Core.Stats` ports them; the update fields show them on the client's
//! character sheet, and [`Derived`] keeps them for combat.

use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::player::Player;

const WARRIOR: u8 = 1;
const PALADIN: u8 = 2;
const HUNTER: u8 = 3;
const ROGUE: u8 = 4;
const PRIEST: u8 = 5;
const SHAMAN: u8 = 7;
const MAGE: u8 = 8;
const WARLOCK: u8 = 9;
const DRUID: u8 = 11;

/// The spells that teach parrying and blocking.
pub const SPELL_PARRY: u32 = 3127;
pub const SPELL_BLOCK: u32 = 107;
/// The spell effects that teach parrying and blocking.
const EFFECT_PARRY: u32 = 22;
const EFFECT_BLOCK: u32 = 23;
pub const SPELL_DUAL_WIELD: u32 = 674;
/// Seal of the Crusader's ranks, 1 to 6.
pub const SEAL_OF_THE_CRUSADER: [u32; 6] = [21082, 20162, 20305, 20306, 20307, 20308];

/// What combat reads of a unit's stats.
#[derive(Debug, Clone, Copy, Default)]
pub struct Derived {
    /// In percent.
    pub crit: f32,
    pub ranged_crit: f32,
    pub dodge: f32,
    pub parry: f32,
    pub block: f32,
    pub spell_crit: [f32; 7],
    /// Each hand's hit chance from auras: the main hand's, the off hand's
    /// and the ranged weapon's.
    pub hit: f32,
    pub off_hit: f32,
    pub ranged_hit: f32,
    pub spell_hit: f32,
    pub block_value: u32,
    pub attack_power: u32,
    pub ranged_attack_power: u32,
    /// Spell power by school, and healing power.
    pub spell_power: [i32; 7],
    pub healing_power: i32,
    /// Each hand's weapon skill and the defense skill, with their bonuses,
    /// as a blow takes them; and against a player (at least their caps).
    pub weapon_skill: u32,
    pub off_skill: u32,
    pub ranged_skill: u32,
    pub defense: u32,
    pub pvp_weapon_skill: u32,
    pub pvp_off_skill: u32,
    pub pvp_ranged_skill: u32,
    pub pvp_defense: u32,
    pub dual_wield: bool,
}

/// A creature's values before its auras.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnitBase {
    pub max_health: u32,
    pub max_mana: u32,
    pub resistances: [i32; 7],
    pub min_damage: f32,
    pub max_damage: f32,
    pub min_ranged: f32,
    pub max_ranged: f32,
    pub attack_time: u32,
    pub ranged_time: u32,
    pub attack_power: u32,
    pub ranged_attack_power: u32,
    pub run_speed: f32,
    /// Strength to spirit: a pet's, which its auras raise (mangos's
    /// `Pet::UpdateStats`).
    pub stats: [u32; 5],
}

impl UnitBase {
    /// A creature's values as spawned.
    pub fn capture(f: &Fields, run_speed: f32) -> Self {
        Self {
            max_health: f.get_u32(UNIT_FIELD_MAXHEALTH),
            max_mana: f.get_u32(UNIT_FIELD_MAXPOWER1),
            resistances: std::array::from_fn(|i| {
                f.get_u32(UNIT_FIELD_RESISTANCES + i as u16) as i32
            }),
            min_damage: f.get_f32(UNIT_FIELD_MINDAMAGE),
            max_damage: f.get_f32(UNIT_FIELD_MAXDAMAGE),
            min_ranged: f.get_f32(UNIT_FIELD_MINRANGEDDAMAGE),
            max_ranged: f.get_f32(UNIT_FIELD_MAXRANGEDDAMAGE),
            attack_time: f.get_u32(UNIT_FIELD_BASEATTACKTIME),
            ranged_time: f.get_u32(UNIT_FIELD_RANGEDATTACKTIME),
            attack_power: f.get_u32(UNIT_FIELD_ATTACK_POWER),
            ranged_attack_power: f.get_u32(UNIT_FIELD_RANGED_ATTACK_POWER),
            run_speed,
            stats: std::array::from_fn(|i| f.get_u32(UNIT_FIELD_STAT0 + i as u16)),
        }
    }
}

/// What a player's gear gives.
#[derive(Debug, Clone, Default)]
pub struct Gear {
    pub stats: [i32; 5],
    pub health: i32,
    pub mana: i32,
    /// Armor, then holy to arcane.
    pub resistances: [i32; 7],
    pub shield: bool,
    pub shield_block: i32,
    pub main: Option<Weapon>,
    pub off: Option<Weapon>,
    pub ranged: Option<Weapon>,
}

/// A weapon in a hand.
#[derive(Debug, Clone, Copy, Default)]
pub struct Weapon {
    pub entry: u32,
    pub guid: Guid,
    pub subclass: u32,
    pub min: f32,
    pub max: f32,
    pub delay: u32,
    pub ammo_type: u32,
}

/// The weapon skill a weapon subclass trains.
pub fn weapon_skill(subclass: u32) -> u32 {
    match subclass {
        0 => 44,   // axes
        1 => 172,  // two-handed axes
        2 => 45,   // bows
        3 => 46,   // guns
        4 => 54,   // maces
        5 => 160,  // two-handed maces
        6 => 229,  // polearms
        7 => 43,   // swords
        8 => 55,   // two-handed swords
        10 => 136, // staves
        13 => 473, // fist weapons
        15 => 173, // daggers
        16 => 176, // thrown
        18 => 226, // crossbows
        19 => 228, // wands
        _ => 162,  // unarmed
    }
}

pub const SKILL_DEFENSE: u32 = 95;

/// What a player's equipped gear gives, and the weapons in their hands.
pub fn gear(p: &Player, data: &GameData) -> Gear {
    use crate::game::store::BAG_PLAYER;
    let mut g = Gear::default();
    for item in p
        .items
        .iter()
        .filter(|i| i.bag == BAG_PLAYER && i.slot < 19)
    {
        let Some(t) = data.item(item.entry) else {
            continue;
        };
        // Broken gear gives nothing.
        if item.fields.get_u32(ITEM_FIELD_MAXDURABILITY) > 0
            && item.fields.get_u32(ITEM_FIELD_DURABILITY) == 0
        {
            continue;
        }
        g.resistances[0] += t.armor;
        for (i, r) in t.resistances.iter().enumerate() {
            g.resistances[i + 1] += r;
        }
        for (kind, value) in t.stats {
            match kind {
                0 => g.mana += value,
                1 => g.health += value,
                3 => g.stats[1] += value,
                4 => g.stats[0] += value,
                5 => g.stats[3] += value,
                6 => g.stats[4] += value,
                7 => g.stats[2] += value,
                _ => {}
            }
        }
        if t.inventory_type == 14 {
            g.shield = true;
            g.shield_block += t.block as i32;
        }
        let weapon = || Weapon {
            entry: t.entry,
            guid: item.guid,
            subclass: t.subclass,
            min: t.damages[0].0,
            max: t.damages[0].1,
            delay: t.delay,
            ammo_type: t.ammo_type,
        };
        if t.class == 2 && t.delay > 0 {
            match item.slot {
                15 => g.main = Some(weapon()),
                16 => g.off = Some(weapon()),
                17 => g.ranged = Some(weapon()),
                _ => {}
            }
        }
        crate::game::map::enchant::add_to_gear(data, item, &mut g);
    }
    g
}

/// Whether a shapeshift form puts the weapons away: cat, travel, aquatic,
/// bear and dire bear form (`SHAPESHIFT_FLAG_DONT_USE_WEAPON` in
/// `SpellShapeshiftForm.dbc`, mangos's `IsNoWeaponShapeShift`). In a
/// stance, Stealth, Shadowform, Moonkin Form or Ghost Wolf, they stay in
/// hand: two are wielded and a blow is parried.
pub fn no_weapon_form(form: u8) -> bool {
    matches!(form, 1 | 3 | 4 | 5 | 8)
}

/// Whether an aura with a weapon restriction applies to `w`.
pub fn fits(h: &Holder, w: Option<&Weapon>) -> bool {
    if h.item_class <= 0 {
        return true;
    }
    let Some(w) = w else {
        return false;
    };
    h.item_class == 2
        && (h.item_subclass_mask <= 0 || h.item_subclass_mask & (1 << w.subclass) != 0)
}

/// The percentages of damage done on school `bit` that apply to the
/// weapon `w` (all those that ask for none, and those it fits).
fn pct_for(a: &Auras, bit: u32, w: Option<&Weapon>) -> f32 {
    a.effects()
        .filter(|(h, x)| {
            x.kind == AURA_MOD_DAMAGE_PERCENT_DONE && (x.misc as u32) & bit != 0 && fits(h, w)
        })
        .fold(1.0, |m, (h, x)| {
            m * (100 + x.amount * h.stacks.max(1) as i32).max(0) as f32 / 100.0
        })
}

/// The physical percentages of damage done that apply to the weapon `w`.
pub fn physical_pct_for(a: &Auras, w: Option<&Weapon>) -> f32 {
    pct_for(a, 1, w)
}

/// The flat physical damage done a player's hand with the weapon `w` takes
/// into its damage, under its percentages: aura 13 of the physical school,
/// one that asks for a weapon only with one it fits, as mangos's
/// `HandleModDamageDone` and `_ApplyWeaponDependentAuraDamageMod` put it in
/// the hand's `TOTAL_VALUE` (Restless Strength's, Curse of Weakness's, a
/// weapon specialization's). The hand's damage fields hold it, so the
/// character sheet shows it and every blow and ability has it.
pub fn physical_flat_for(a: &Auras, w: Option<&Weapon>) -> f32 {
    a.effects()
        .filter(|(h, x)| x.kind == AURA_MOD_DAMAGE_DONE && x.misc & 1 != 0 && fits(h, w))
        .map(|(h, x)| (x.amount * h.stacks.max(1) as i32) as f32)
        .sum()
}

/// A spell's percentages of damage done on school `bit`: those that ask
/// for no weapon or item.
pub fn spell_pct(a: &Auras, bit: u32) -> f32 {
    a.effects()
        .filter(|(h, x)| {
            x.kind == AURA_MOD_DAMAGE_PERCENT_DONE && (x.misc as u32) & bit != 0 && h.unrestricted()
        })
        .fold(1.0, |m, (h, x)| {
            m * (100 + x.amount * h.stacks.max(1) as i32).max(0) as f32 / 100.0
        })
}

/// A wand's: weapon subclass 19.
pub const SUBCLASS_WAND: u32 = 19;

/// What a wand's shot takes of damage done: the percentages of its school
/// (`school_bit`) on auras that fit it, Wand Specialization's among them,
/// each by its stacks (Thistle Tea's wand context and displayed damage).
pub fn wand_pct(a: &Auras, school_bit: u32, w: &Weapon) -> f32 {
    a.effects()
        .filter(|(h, x)| {
            x.kind == AURA_MOD_DAMAGE_PERCENT_DONE
                && (x.misc as u32) & school_bit != 0
                && fits(h, Some(w))
        })
        .fold(1.0, |m, (h, x)| {
            m * (100 + x.amount * h.stacks.max(1) as i32).max(0) as f32 / 100.0
        })
}

/// The sum of `kind` auras that apply to the weapon `w`.
fn sum_for(a: &Auras, kind: u32, w: Option<&Weapon>) -> f32 {
    a.effects()
        .filter(|(h, x)| x.kind == kind && fits(h, w))
        .map(|(h, x)| (x.amount * h.stacks.max(1) as i32) as f32)
        .sum()
}

/// A haste or slow as a multiplier on a period: +25 haste is 0.8.
fn speed_factor(amount: i32) -> f32 {
    if amount >= 0 {
        100.0 / (100 + amount) as f32
    } else {
        (100 - amount) as f32 / 100.0
    }
}

/// The hastes and slows of `kinds`, each a factor on the period: they all
/// multiply, the slows too (mangos-classic's `ApplyAttackTimePercentMod`,
/// the user's #30; Thistle Tea's `AttackSpeed.multiplier` takes only the
/// strongest melee slow).
pub(crate) fn period_multiplier(a: &Auras, kinds: &[u32]) -> f32 {
    a.effects()
        .filter(|(_, x)| kinds.contains(&x.kind))
        .fold(1.0, |m, (h, x)| {
            m * speed_factor(x.amount * h.stacks.max(1) as i32)
        })
}

/// What a unit's melee hastes and slows make of its swing's period.
pub fn melee_haste(a: &Auras) -> f32 {
    period_multiplier(a, &[AURA_MOD_HASTE, AURA_MOD_ATTACKSPEED])
}

/// Per-class rates for crit and dodge from agility at levels 1 and 60,
/// and between them on a line.
pub fn agility_rate(class: u8, level: u32, dodge: bool) -> f32 {
    let (l1, l60) = match (class, dodge) {
        (WARRIOR, _) => (3.9, 20.0),
        (PALADIN, _) => (4.6, 20.0),
        (HUNTER, false) => (3.5, 53.0),
        (HUNTER, true) => (1.8, 26.5),
        (ROGUE, false) => (2.2, 29.0),
        (ROGUE, true) => (1.1, 14.5),
        (PRIEST, _) => (11.0, 20.0),
        (SHAMAN, _) => (4.6, 20.0),
        (MAGE, _) => (12.9, 20.0),
        (WARLOCK, _) => (8.4, 20.0),
        (DRUID, _) => (4.6, 20.0),
        _ => (20.0, 20.0),
    };
    let level = level.clamp(1, 60) as f32;
    l1 * (60.0 - level) / 59.0 + l60 * (level - 1.0) / 59.0
}

/// A class's crit and dodge before agility.
pub fn class_base_bonus(class: u8) -> f32 {
    match class {
        PALADIN => 0.7,
        PRIEST => 3.0,
        SHAMAN => 1.7,
        MAGE => 3.2,
        WARLOCK => 2.0,
        DRUID => 0.9,
        _ => 0.0,
    }
}

/// Spell crit from intellect.
pub fn spell_crit_chance(class: u8, level: u32, intellect: u32) -> f32 {
    let (base, r0, r1) = match class {
        PALADIN => (3.70, 14.77, 0.65),
        PRIEST => (2.97, 10.03, 0.82),
        SHAMAN => (3.54, 11.51, 0.80),
        MAGE => (3.70, 14.77, 0.65),
        WARLOCK => (3.18, 11.30, 0.82),
        DRUID => (3.33, 12.41, 0.79),
        _ => return 0.0,
    };
    (base + intellect as f32 / (r0 + r1 * level.max(1) as f32)).max(0.0)
}

pub fn melee_attack_power(class: u8, level: u32, strength: u32, agility: u32) -> u32 {
    stat_melee_attack_power(class, level, strength, agility).max(0) as u32
}

/// A class's melee attack power from its stats, unclamped: a creature
/// takes the change of it (Thistle Tea's `stat_melee_attack_power`).
fn stat_melee_attack_power(class: u8, level: u32, strength: u32, agility: u32) -> i32 {
    let (l, s, a) = (level as i32, strength as i32, agility as i32);
    match class {
        WARRIOR | PALADIN => l * 3 + s * 2 - 20,
        ROGUE | HUNTER => l * 2 + s + a - 20,
        SHAMAN => l * 2 + s * 2 - 20,
        // In every form (mangos-classic's `UpdateAttackPowerAndDamage`):
        // the forms' own share is their passive auras'.
        DRUID => l * 3 + s * 2 - 20,
        _ => s - 10,
    }
}

pub fn ranged_attack_power(class: u8, level: u32, agility: u32) -> u32 {
    stat_ranged_attack_power(class, level, agility).max(0) as u32
}

fn stat_ranged_attack_power(class: u8, level: u32, agility: u32) -> i32 {
    let (l, a) = (level as i32, agility as i32);
    match class {
        HUNTER => l * 2 + a * 2 - 10,
        ROGUE | WARRIOR => l + a - 10,
        _ => a - 10,
    }
}

/// Health from stamina: one a point for the first twenty, ten after.
pub fn stamina_health(stamina: u32) -> u32 {
    stamina.min(20) + stamina.saturating_sub(20) * 10
}

fn intellect_mana(intellect: u32) -> u32 {
    intellect.min(20) + intellect.saturating_sub(20) * 15
}

/// The percentages of `kind` on stat `misc` (or all, -1), each `100 +
/// amount` with its stacks, no less than nothing.
fn factors(a: &Auras, kind: u32, misc: i32) -> Vec<i64> {
    a.effects()
        .filter(|(_, x)| x.kind == kind && (x.misc == misc || x.misc == -1))
        .map(|(h, x)| i64::from((100 + x.amount * h.stacks.max(1) as i32).max(0)))
        .collect()
}

/// A stat from its base and gear (`own`): scaled by the base percentages
/// (`pct`, each `100 + amount`), the flat auras added, then scaled by the
/// totals, in whole numbers as Thistle Tea's `scaled_stat` divides them:
/// four percent of a hundred is four, where a float's 1.04 makes 103.99.
pub fn scaled_stat(own: i32, pct: &[i64], flat: i32, total: &[i64]) -> u32 {
    let (mut num, mut den) = (1i128, 1i128);
    for f in pct {
        num *= i128::from(*f);
        den *= 100;
    }
    let (mut tnum, mut tden) = (1i128, 1i128);
    for f in total {
        tnum *= i128::from(*f);
        tden *= 100;
    }
    let value = i128::from(own) * num + i128::from(flat) * den;
    (value * tnum / (den * tden)).clamp(0, i128::from(u32::MAX)) as u32
}

/// `SPELL_AURA_MOD_RESISTANCE_OF_STAT_PERCENT`: a share of intellect (the
/// only stat it takes in 1.12) as armor or resistance.
const AURA_MOD_RESISTANCE_OF_STAT_PERCENT: u32 = 182;

/// The percentages of `kind` on school `bit`, each a factor, in doubles
/// as Thistle Tea multiplies them: 1.4 and 1.5 make 2.1, not 2.0999999.
fn percent_product(a: &Auras, kind: u32, bit: u32) -> f64 {
    a.effects()
        .filter(|(_, x)| x.kind == kind && (x.misc as u32) & bit != 0)
        .fold(1.0, |m, (h, x)| {
            m * f64::from((100 + x.amount * h.stacks.max(1) as i32).max(0)) / 100.0
        })
}

/// Armor (school 0) and resistances, from a base, auras, agility and
/// intellect, as Thistle Tea's `Resistances.recompute`: the base and its
/// flats scaled by the base percentages; agility's armor, intellect's
/// share and the flat bonuses on top; then the percentages, truncated
/// once at the end. Agility gives `per_agility` armor a point: a player's
/// and a hunter's pet's two, another creature's one.
fn resistances(
    a: &Auras,
    base: [i32; 7],
    agility: u32,
    intellect: u32,
    per_agility: u32,
) -> [i32; 7] {
    std::array::from_fn(|i| {
        let bit = 1u32 << i;
        let initial = base[i] + a.sum_mask(AURA_MOD_BASE_RESISTANCE, bit);
        let scaled = f64::from(initial) * percent_product(a, AURA_MOD_BASE_RESISTANCE_PCT, bit);
        let stat = f64::from(if i == 0 { agility * per_agility } else { 0 });
        let exclusive = {
            let v: Vec<i32> = a
                .effects()
                .filter(|(_, x)| {
                    x.kind == AURA_MOD_RESISTANCE_EXCLUSIVE && (x.misc as u32) & bit != 0
                })
                .map(|(h, x)| x.amount * h.stacks.max(1) as i32)
                .collect();
            v.iter().copied().max().unwrap_or(0).max(0)
                + v.iter().copied().min().unwrap_or(0).min(0)
        };
        let of_intellect = f64::from(intellect)
            * f64::from(a.sum_mask(AURA_MOD_RESISTANCE_OF_STAT_PERCENT, bit))
            / 100.0;
        let total = scaled
            + stat
            + of_intellect
            + f64::from(a.sum_mask(AURA_MOD_RESISTANCE, bit))
            + f64::from(exclusive);
        let value = (total * percent_product(a, AURA_MOD_RESISTANCE_PCT, bit)) as i32;
        if base[i] < 0 { value } else { value.max(0) }
    })
}

/// `UNIT_FIELD_AURASTATE`'s Judgement bit (aura state 5): set while a seal
/// is on, as mangos's seals set `AURA_STATE_JUDGEMENT`, so a Judgement
/// (caster aura state 5) may be cast, and its button lit.
const AURA_STATE_JUDGEMENT_BIT: u32 = 1 << 4;

fn judgement_state(p: &mut Player) {
    let sealed = p
        .auras
        .holders
        .iter()
        .any(|h| h.exclusive == Some(auras::Exclusive::PaladinSeal));
    let state = p.fields.get_u32(UNIT_FIELD_AURASTATE);
    let want = if sealed {
        state | AURA_STATE_JUDGEMENT_BIT
    } else {
        state & !AURA_STATE_JUDGEMENT_BIT
    };
    if want != state {
        p.fields.set_u32(UNIT_FIELD_AURASTATE, want);
    }
}

/// Work a unit's stats out again.
pub fn unit(map: &mut Map, g: Guid) {
    let data = map.realm.data.clone();
    if let Some(p) = map.players.get_mut(&g) {
        player(p, &data);
        judgement_state(p);
        speed(map, g);
        return;
    }
    if let Some(c) = map.creature_mut(g) {
        creature(c);
        speed(map, g);
    }
}

/// A player's stats from their level, gear and auras.
pub fn player(p: &mut Player, data: &GameData) {
    let level = u32::from(p.level());
    let class = p.row.class;
    let Some(base) = data.level_stats(p.row.race, class, p.level()) else {
        p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 100);
        return;
    };
    crate::game::map::skills::set_bonuses(p);
    let g = gear(p, data);
    let form = p.fields.get_byte(UNIT_FIELD_BYTES_1, 2);
    let disarmed = p.auras.has(AURA_MOD_DISARM);
    // A skill as a blow takes it, with its bonuses, a negative one too, to
    // no less than 0 (mangos's `GetSkillValue`); and against a player, no
    // less than its cap with them (`GetWeaponSkillValue` and
    // `GetDefenseSkillValue` in PvP).
    let skill_values = |skill: u32| {
        let bonus = p.auras.sum_misc(AURA_MOD_SKILL, skill as i32)
            + p.auras.sum_misc(AURA_MOD_SKILL_TALENT, skill as i32);
        let value = crate::game::map::skills::value(p, skill).unwrap_or(level * 5) as i32 + bonus;
        let max = crate::game::map::skills::max_value(p, skill).unwrap_or(level * 5) as i32 + bonus;
        (value.max(0) as u32, value.max(max).max(0) as u32)
    };
    let skill_of = |w: Option<&Weapon>| skill_values(w.map_or(162, |w| weapon_skill(w.subclass)));
    let feral = class == DRUID
        && matches!(
            form,
            auras::FORM_CAT | auras::FORM_BEAR | auras::FORM_DIREBEAR
        );
    // In cat or bear form the paws fight at the level's cap; an off hand or
    // a ranged slot without a weapon has no skill (`GetWeaponSkillValue`).
    let (weapon_skill, pvp_weapon_skill) = if feral {
        (level * 5, level * 5)
    } else {
        skill_of(g.main.filter(|_| !disarmed).as_ref())
    };
    let (off_skill, pvp_off_skill) = g.off.map_or((0, 0), |w| skill_of(Some(&w)));
    let (ranged_skill, pvp_ranged_skill) = g.ranged.map_or((0, 0), |w| skill_of(Some(&w)));
    let (defense, pvp_defense) = skill_values(SKILL_DEFENSE);
    // The weapons in the hands, broken, disarmed or in a form or not: the
    // hit chances that want a weapon go by them (mangos's
    // `UpdateWeaponHitChances`, `GetWeaponForAttack` of any).
    let held = |slot: u8| {
        p.items
            .iter()
            .find(|i| i.bag == crate::game::store::BAG_PLAYER && i.slot == slot)
            .and_then(|i| data.item(i.entry))
            .filter(|t| t.class == 2 && t.delay > 0)
            .map(|t| Weapon {
                entry: t.entry,
                subclass: t.subclass,
                ..Default::default()
            })
    };
    let held = [held(15), held(16), held(17)];
    // Any spell of Parry's or Block's effect teaches it (mangos's
    // `EffectParry` and `EffectBlock`, Thistle Tea's `Proficiency`).
    let knows = |id: u32, effect: u32| {
        p.spells.iter().any(|s| {
            *s == id
                || data
                    .spells
                    .get(*s)
                    .is_some_and(|t| t.effects.iter().any(|e| e.effect == effect))
        })
    };
    let knows_parry = knows(SPELL_PARRY, EFFECT_PARRY);
    let knows_block = knows(SPELL_BLOCK, EFFECT_BLOCK);
    let knows_dual_wield = p.spells.contains(&SPELL_DUAL_WIELD);
    let a = &p.auras;
    let f = &mut p.fields;
    let mut stat = [0u32; 5];
    for (i, value) in stat.iter_mut().enumerate() {
        let own = base.stats[i] as i32 + g.stats[i];
        *value = scaled_stat(
            own,
            &factors(a, AURA_MOD_PERCENT_STAT, i as i32),
            a.sum_misc(AURA_MOD_STAT, i as i32),
            &factors(a, AURA_MOD_TOTAL_STAT_PERCENTAGE, i as i32),
        );
        let s = i as u16;
        f.set_u32(UNIT_FIELD_STAT0 + s, *value);
        let diff = *value as i32 - base.stats[i] as i32;
        f.set_u32(PLAYER_FIELD_POSSTAT0 + s, diff.max(0) as u32);
        f.set_u32(PLAYER_FIELD_NEGSTAT0 + s, (-diff).max(0) as u32);
    }
    let [strength, agility, stamina, intellect, spirit] = stat;
    let _ = spirit;
    // Health and power.
    let health = (base.health + stamina_health(stamina)) as f32
        + g.health as f32
        + a.sum(AURA_MOD_INCREASE_HEALTH) as f32;
    let health = (health * a.product(AURA_MOD_INCREASE_HEALTH_PERCENT)).max(1.0) as u32;
    f.set_u32(UNIT_FIELD_BASE_HEALTH, base.health);
    f.set_u32(UNIT_FIELD_MAXHEALTH, health);
    if f.get_u32(UNIT_FIELD_HEALTH) > health {
        f.set_u32(UNIT_FIELD_HEALTH, health);
    }
    f.set_u32(UNIT_FIELD_BASE_MANA, base.mana);
    if base.mana > 0 {
        let mana = (base.mana + intellect_mana(intellect)) as f32
            + g.mana as f32
            + a.sum_misc(AURA_MOD_INCREASE_ENERGY, 0) as f32;
        let mana = (mana * a.product_misc(AURA_MOD_INCREASE_ENERGY_PERCENT, 0)).max(0.0) as u32;
        f.set_u32(UNIT_FIELD_MAXPOWER1, mana);
        if f.get_u32(UNIT_FIELD_POWER1) > mana {
            f.set_u32(UNIT_FIELD_POWER1, mana);
        }
    }
    let energy = (100.0 + a.sum_misc(AURA_MOD_INCREASE_ENERGY, 3) as f32)
        * a.product_misc(AURA_MOD_INCREASE_ENERGY_PERCENT, 3);
    if class == ROGUE || class == DRUID {
        f.set_u32(UNIT_FIELD_MAXPOWER1 + 3, energy.max(0.0) as u32);
    }
    if class == WARRIOR || class == DRUID {
        f.set_u32(UNIT_FIELD_MAXPOWER1 + 1, 1000);
    }
    // Armor and resistances.
    let res = resistances(a, g.resistances, agility, intellect, 2);
    for (i, v) in res.iter().enumerate() {
        let s = i as u16;
        f.set_u32(UNIT_FIELD_RESISTANCES + s, (*v).max(0) as u32);
        let diff = *v - g.resistances[i];
        f.set_u32(
            PLAYER_FIELD_RESISTANCEBUFFMODSPOSITIVE + s,
            diff.max(0) as u32,
        );
        f.set_u32(
            PLAYER_FIELD_RESISTANCEBUFFMODSNEGATIVE + s,
            (-diff).max(0) as u32,
        );
    }
    // Attack power.
    let mut ap_base = melee_attack_power(class, level, strength, agility);
    if class == DRUID {
        // Cat form adds agility, the one form flagged so in
        // SpellShapeshiftForm (`SHAPESHIFT_FLAG_AGILITY_ATTACK_BONUS`); a
        // form's level share is its passive aura's (Cat Form (Passive)'s
        // 40 and two a level, Bear Form's 30 and three), as
        // mangos-classic has it, not here.
        if form == auras::FORM_CAT {
            ap_base += agility;
        }
        // Predatory Strikes: half, once or one and a half times the level
        // more in cat and bear form (mangos's
        // `spell_druid_form_scaling_ap_auras` on the forms' passives).
        if matches!(
            form,
            auras::FORM_CAT | auras::FORM_BEAR | auras::FORM_DIREBEAR
        ) {
            let pct = a
                .effects()
                .filter(|(h, x)| matches!(h.spell, 16972 | 16974 | 16975) && x.kind == AURA_DUMMY)
                .map(|(_, x)| x.amount)
                .max()
                .unwrap_or(0);
            ap_base += level * pct.max(0) as u32 / 100;
        }
    }
    let ap_mods = a.sum(AURA_MOD_ATTACK_POWER);
    let ap_mult = a.product(AURA_MOD_ATTACK_POWER_PCT);
    f.set_u32(UNIT_FIELD_ATTACK_POWER, ap_base);
    f.set_u32(UNIT_FIELD_ATTACK_POWER_MODS, ap_mods.max(0) as u32);
    f.set_f32(UNIT_FIELD_ATTACK_POWER_MULTIPLIER, ap_mult - 1.0);
    let ap = ((ap_base as i32 + ap_mods) as f32 * ap_mult).max(0.0);
    // A druid in Cat or Bear form has none ranged (mangos-classic's).
    let rap_base = if class == DRUID
        && matches!(
            form,
            auras::FORM_CAT | auras::FORM_BEAR | auras::FORM_DIREBEAR
        ) {
        0
    } else {
        ranged_attack_power(class, level, agility)
    };
    let rap_mods = a.sum(AURA_MOD_RANGED_ATTACK_POWER);
    let rap_mult = a.product(AURA_MOD_RANGED_ATTACK_POWER_PCT);
    f.set_u32(UNIT_FIELD_RANGED_ATTACK_POWER, rap_base);
    f.set_u32(UNIT_FIELD_RANGED_ATTACK_POWER_MODS, rap_mods.max(0) as u32);
    f.set_f32(UNIT_FIELD_RANGED_ATTACK_POWER_MULTIPLIER, rap_mult - 1.0);
    let rap = ((rap_base as i32 + rap_mods) as f32 * rap_mult).max(0.0);
    // Weapon damage and speed: a druid in cat or bear form fights with
    // the form's paws.
    // A percentage that asks for a weapon (a weapon specialization's)
    // counts for the hand whose weapon fits it, as mangos's
    // `_ApplyWeaponDependentAuraDamageMod`.
    let physical_pct = |w: Option<&Weapon>| pct_for(a, 1, w);
    let (main, main_delay) = match (class, form) {
        (DRUID, auras::FORM_CAT) => (None, 1000),
        (DRUID, auras::FORM_BEAR | auras::FORM_DIREBEAR) => (None, 2500),
        _ => match g.main.filter(|_| !disarmed) {
            Some(w) => (Some(w), w.delay),
            None => (None, 2000),
        },
    };
    let (min, max) = main.map_or((1.0, 2.0), |w| (w.min, w.max));
    let (min, max) = if main.is_none() && class == DRUID && form != 0 {
        // A level-scaled paw.
        let base = level as f32 * 0.85 * main_delay as f32 / 1000.0;
        (base.max(1.0), (base * 1.25).max(2.0))
    } else {
        (min, max)
    };
    // Seal of the Crusader's dummy: the main hand's own damage over 1.4, so
    // its 40% faster swings deal no more (mangos-classic's `HandleAuraDummy`,
    // `m_modAttackBaseDPSPct[BASE_ATTACK]`, before attack power's bonus).
    let crusader = a
        .holders
        .iter()
        .filter(|h| SEAL_OF_THE_CRUSADER.contains(&h.spell))
        .count();
    let (min, max) = {
        let pct = (100.0f32 / 140.0).powi(crusader as i32);
        (min * pct, max * pct)
    };
    let haste = period_multiplier(a, &[AURA_MOD_HASTE, AURA_MOD_ATTACKSPEED]);
    let bonus = ap / 14.0 * main_delay as f32 / 1000.0;
    let main_pct = physical_pct(main.as_ref());
    let bonus = bonus + physical_flat_for(a, main.as_ref());
    f.set_f32(UNIT_FIELD_MINDAMAGE, (min + bonus) * main_pct);
    f.set_f32(UNIT_FIELD_MAXDAMAGE, (max + bonus) * main_pct);
    f.set_u32(
        UNIT_FIELD_BASEATTACKTIME,
        ((main_delay as f32 * haste) as u32).max(100),
    );
    let dual_wield = knows_dual_wield && g.off.is_some() && !no_weapon_form(form);
    match g.off.filter(|_| dual_wield) {
        Some(w) => {
            let bonus = ap / 14.0 * w.delay as f32 / 1000.0 + physical_flat_for(a, Some(&w));
            let pct =
                physical_pct(Some(&w)) * (1.0 + a.sum(AURA_MOD_OFFHAND_DAMAGE_PCT) as f32 / 100.0);
            f.set_f32(UNIT_FIELD_MINOFFHANDDAMAGE, (w.min + bonus) * 0.5 * pct);
            f.set_f32(UNIT_FIELD_MAXOFFHANDDAMAGE, (w.max + bonus) * 0.5 * pct);
            f.set_u32(
                UNIT_FIELD_BASEATTACKTIME + 1,
                ((w.delay as f32 * haste) as u32).max(100),
            );
        }
        None => {
            f.set_f32(UNIT_FIELD_MINOFFHANDDAMAGE, 0.0);
            f.set_f32(UNIT_FIELD_MAXOFFHANDDAMAGE, 0.0);
            f.set_u32(UNIT_FIELD_BASEATTACKTIME + 1, 2000);
        }
    }
    match g.ranged {
        // A wand: its own damage and school, without attack power, by its
        // school's percentages that fit it (Thistle Tea's wand).
        Some(w) if w.subclass == SUBCLASS_WAND => {
            let school = data.item(w.entry).map_or(0, |t| t.damages[0].2).min(6);
            let pct = wand_pct(a, 1 << school, &w);
            f.set_f32(UNIT_FIELD_MINRANGEDDAMAGE, w.min * pct);
            f.set_f32(UNIT_FIELD_MAXRANGEDDAMAGE, w.max * pct);
            let ranged_haste = period_multiplier(a, &[AURA_MOD_RANGED_HASTE, AURA_MOD_ATTACKSPEED]);
            f.set_u32(
                UNIT_FIELD_RANGEDATTACKTIME,
                ((w.delay as f32 * ranged_haste) as u32).max(100),
            );
        }
        Some(w) => {
            let ammo = if w.ammo_type != 0 {
                let id = f.get_u32(PLAYER_AMMO_ID);
                data.item(id)
                    .map(|t| (t.damages[0].0 + t.damages[0].1) / 2.0)
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            let bonus =
                (rap / 14.0 + ammo) * w.delay as f32 / 1000.0 + physical_flat_for(a, Some(&w));
            let pct = physical_pct(Some(&w));
            f.set_f32(UNIT_FIELD_MINRANGEDDAMAGE, (w.min + bonus) * pct);
            f.set_f32(UNIT_FIELD_MAXRANGEDDAMAGE, (w.max + bonus) * pct);
            let mut kinds = vec![AURA_MOD_RANGED_HASTE, AURA_MOD_ATTACKSPEED];
            if w.ammo_type != 0 {
                kinds.push(AURA_MOD_RANGED_AMMO_HASTE);
            }
            let ranged_haste = period_multiplier(a, &kinds);
            f.set_u32(
                UNIT_FIELD_RANGEDATTACKTIME,
                ((w.delay as f32 * ranged_haste) as u32).max(100),
            );
        }
        None => {
            f.set_f32(UNIT_FIELD_MINRANGEDDAMAGE, 0.0);
            f.set_f32(UNIT_FIELD_MAXRANGEDDAMAGE, 0.0);
            f.set_u32(UNIT_FIELD_RANGEDATTACKTIME, 2000);
        }
    }
    let cast = period_multiplier(a, &[AURA_MOD_CASTING_SPEED]);
    f.set_f32(UNIT_MOD_CAST_SPEED, cast);
    // Spell power from spirit's share (aura 174).
    let spirit_power = |school: u32| {
        let pct = a.sum_mask(AURA_MOD_SPELL_DAMAGE_OF_STAT_PERCENT, 1 << school);
        (spirit as i32 * pct / 100).max(0)
    };
    // Damage done, as the client shows it: spirit's share in it too (mangos's
    // `UpdateSpellDamageBonus`, Thistle Tea's `TemporarySpellPower`).
    for i in 0..7u16 {
        let bit = 1u32 << i;
        let (pos, neg) = a
            .effects()
            .filter(|(h, x)| {
                x.kind == AURA_MOD_DAMAGE_DONE && (x.misc as u32) & bit != 0 && h.unrestricted()
            })
            .fold((0i32, 0i32), |(p, n), (h, x)| {
                let v = x.amount * h.stacks.max(1) as i32;
                if v >= 0 { (p + v, n) } else { (p, n - v) }
            });
        f.set_u32(
            PLAYER_FIELD_MOD_DAMAGE_DONE_POS + i,
            (pos + spirit_power(u32::from(i))) as u32,
        );
        f.set_u32(PLAYER_FIELD_MOD_DAMAGE_DONE_NEG + i, neg as u32);
        // A spell's share: none that asks for a weapon (mangos's
        // `SpellDamageBonusDone` leaves those out).
        f.set_f32(PLAYER_FIELD_MOD_DAMAGE_DONE_PCT + i, spell_pct(a, bit));
    }
    // The chances.
    let max_skill = level * 5;
    // Melee crit from agility at vmangos's rate for the class and level,
    // where `player_crit_per_agility` has one; else on the line.
    let crit_rate = data
        .misc
        .crit_per_agility(class, level as u8)
        .unwrap_or_else(|| agility_rate(class, level, false));
    let base_crit = class_base_bonus(class) + agility as f32 / crit_rate;
    let crit = (base_crit
        + sum_for(a, AURA_MOD_CRIT_PERCENT, main.as_ref())
        + (weapon_skill as f32 - max_skill as f32) * 0.04)
        .max(0.0);
    let ranged_crit = (base_crit
        + sum_for(a, AURA_MOD_CRIT_PERCENT, g.ranged.as_ref())
        + (ranged_skill as f32 - max_skill as f32) * 0.04)
        .max(0.0);
    let defense_bonus = (defense as f32 - max_skill as f32) * 0.04;
    let dodge = (class_base_bonus(class)
        + agility as f32 / agility_rate(class, level, true)
        + a.sum(AURA_MOD_DODGE_PERCENT) as f32
        + defense_bonus)
        .max(0.0);
    // Parrying wants a melee weapon in a hand (mangos's
    // `CanParryInCombat`, Thistle Tea's `parry_chance`).
    let armed = g.main.is_some() || g.off.is_some();
    let can_parry = knows_parry && !disarmed && !no_weapon_form(form) && armed;
    let parry = if can_parry {
        (5.0 + a.sum(AURA_MOD_PARRY_PERCENT) as f32 + defense_bonus).max(0.0)
    } else {
        0.0
    };
    let can_block = knows_block && g.shield;
    let block = if can_block {
        (5.0 + a.sum(AURA_MOD_BLOCK_PERCENT) as f32 + defense_bonus).max(0.0)
    } else {
        0.0
    };
    let block_value = if can_block {
        let flat = g.shield_block as f32 + a.sum(AURA_MOD_SHIELD_BLOCKVALUE) as f32;
        ((flat + strength as f32 / 20.0 - 1.0) * a.product(AURA_MOD_SHIELD_BLOCKVALUE_PCT)).max(0.0)
            as u32
    } else {
        0
    };
    f.set_f32(PLAYER_CRIT_PERCENTAGE, crit);
    f.set_f32(PLAYER_RANGED_CRIT_PERCENTAGE, ranged_crit);
    f.set_f32(PLAYER_DODGE_PERCENTAGE, dodge);
    f.set_f32(PLAYER_PARRY_PERCENTAGE, parry);
    f.set_f32(PLAYER_BLOCK_PERCENTAGE, block);
    let base_spell_crit = spell_crit_chance(class, level, intellect);
    let spell_crit = std::array::from_fn(|i| {
        base_spell_crit
            + a.sum(AURA_MOD_SPELL_CRIT_CHANCE) as f32
            + a.sum_mask(AURA_MOD_SPELL_CRIT_CHANCE_SCHOOL, 1 << i) as f32
    });
    let spell_power = std::array::from_fn(|i| {
        a.effects()
            .filter(|(h, x)| {
                x.kind == AURA_MOD_DAMAGE_DONE
                    && (x.misc as u32) & (1 << i) != 0
                    && h.unrestricted()
            })
            .map(|(h, x)| x.amount * h.stacks.max(1) as i32)
            .sum::<i32>()
            + spirit_power(i as u32)
    });
    let healing_power = a.sum(AURA_MOD_HEALING_DONE)
        + (spirit as i32 * a.sum(AURA_MOD_SPELL_HEALING_OF_STAT_PERCENT) / 100).max(0);
    p.derived = Derived {
        crit,
        ranged_crit,
        dodge,
        parry,
        block,
        spell_crit,
        hit: sum_for(a, AURA_MOD_HIT_CHANCE, held[0].as_ref()),
        off_hit: sum_for(a, AURA_MOD_HIT_CHANCE, held[1].as_ref()),
        ranged_hit: sum_for(a, AURA_MOD_HIT_CHANCE, held[2].as_ref()),
        spell_hit: a.sum(AURA_MOD_SPELL_HIT_CHANCE) as f32,
        block_value,
        attack_power: ap as u32,
        ranged_attack_power: rap as u32,
        spell_power,
        healing_power,
        weapon_skill,
        off_skill,
        ranged_skill,
        defense,
        pvp_weapon_skill,
        pvp_off_skill,
        pvp_ranged_skill,
        pvp_defense,
        dual_wield,
    };
}

/// How a creature's attack power comes from its stats (Thistle Tea's
/// `attack_power_model`): a hunter's pet's, a demon's (the imp's apart)
/// and any other creature's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerModel {
    Creature,
    HunterPet,
    Demon,
    Imp,
}

/// The imp, whose attack power is its strength - 10.
const IMP: u32 = 416;

impl PowerModel {
    fn of(c: &crate::game::map::creature::Creature) -> Self {
        use crate::game::map::pets::Kind;
        match c.pet.as_deref().map(|s| s.kind) {
            Some(Kind::Hunter) => PowerModel::HunterPet,
            Some(Kind::Demon) if c.entry == IMP => PowerModel::Imp,
            Some(Kind::Demon) => PowerModel::Demon,
            _ => PowerModel::Creature,
        }
    }
}

/// A creature's stats from its spawn values and auras.
pub fn creature(c: &mut crate::game::map::creature::Creature) {
    let b = c.base;
    let a = &c.auras;
    let model = PowerModel::of(c);
    let f = &mut c.fields;
    // Its stats, from its class's at its level (a pet's from its own
    // table) through its auras; the change of them moves its health,
    // mana, armor and attack power (Thistle Tea's creature stat model,
    // mangos-classic's `Creature::UpdateStats`).
    let mut stats = [0u32; 5];
    for (i, stat) in stats.iter_mut().enumerate() {
        *stat = scaled_stat(
            b.stats[i] as i32,
            &factors(a, AURA_MOD_PERCENT_STAT, i as i32),
            a.sum_misc(AURA_MOD_STAT, i as i32),
            &factors(a, AURA_MOD_TOTAL_STAT_PERCENTAGE, i as i32),
        );
        f.set_u32(UNIT_FIELD_STAT0 + i as u16, *stat);
    }
    let change = |i: usize| stats[i] as f32 - b.stats[i] as f32;
    // Ten health a point of stamina it gains, fifteen mana a point of
    // intellect with mana already (mangos's `Unit::UpdateMaxPower`
    // "ignore classes without mana").
    if b.max_mana > 0 {
        let mana = (b.max_mana as f32 + change(3) * 15.0).max(0.0) as u32;
        f.set_u32(UNIT_FIELD_MAXPOWER1, mana);
        if f.get_u32(UNIT_FIELD_POWER1) > mana {
            f.set_u32(UNIT_FIELD_POWER1, mana);
        }
    }
    let health = ((b.max_health as f32 + change(2) * 10.0 + a.sum(AURA_MOD_INCREASE_HEALTH) as f32)
        * a.product(AURA_MOD_INCREASE_HEALTH_PERCENT))
    .max(1.0) as u32;
    f.set_u32(UNIT_FIELD_MAXHEALTH, health);
    if f.get_u32(UNIT_FIELD_HEALTH) > health {
        f.set_u32(UNIT_FIELD_HEALTH, health);
    }
    let per_agility = if model == PowerModel::HunterPet { 2 } else { 1 };
    let res = resistances(a, b.resistances, stats[1], stats[3], per_agility);
    for (i, v) in res.iter().enumerate() {
        f.set_u32(UNIT_FIELD_RESISTANCES + i as u16, (*v).max(0) as u32);
    }
    // Attack power: a pet's from its strength as it is (mangos-classic's
    // `Pet::UpdateAttackPowerAndDamage`: the imp's strength - 10, the
    // others' 2 x strength - 20); another creature's from its class and
    // level's, moved by the change of its stats' share (Thistle Tea's
    // `creature_stat_attack_power`, unclamped).
    let class = f.get_byte(UNIT_FIELD_BYTES_0, 1);
    let level = f.get_u32(UNIT_FIELD_LEVEL);
    let (str_, agi) = (stats[0], stats[1]);
    let pet_base = match model {
        PowerModel::Imp => Some(str_ as i32 - 10),
        PowerModel::HunterPet | PowerModel::Demon => Some(str_ as i32 * 2 - 20),
        PowerModel::Creature => None,
    };
    let melee_base = pet_base.unwrap_or_else(|| {
        b.attack_power as i32 + stat_melee_attack_power(class, level, str_, agi)
            - stat_melee_attack_power(class, level, b.stats[0], b.stats[1])
    });
    let ap_mods = a.sum(AURA_MOD_ATTACK_POWER);
    let ap = ((melee_base + ap_mods) as f32 * a.product(AURA_MOD_ATTACK_POWER_PCT)).max(0.0);
    f.set_u32(UNIT_FIELD_ATTACK_POWER, melee_base.max(0) as u32);
    f.set_u32(UNIT_FIELD_ATTACK_POWER_MODS, ap_mods.max(0) as u32);
    // vmangos's creature damage with attack power changed: seven tenths
    // its own, three tenths the new. A hunter's pet and other creatures
    // weigh it against their level's; a demon against its strength's, so
    // its strength moves its attack power alone (Thistle Tea's
    // `creature_multiplier`).
    let seed = match model {
        PowerModel::Imp | PowerModel::Demon => pet_base.unwrap_or(0).max(0) as f32,
        _ => b.attack_power as f32,
    };
    let ratio = if seed > 0.0 {
        (7.0 * seed + 3.0 * ap) / (10.0 * seed)
    } else {
        1.0 + (ap_mods as f32 / 14.0) / b.max_damage.max(1.0)
    };
    let pct = a.product_mask(AURA_MOD_DAMAGE_PERCENT_DONE, 1);
    f.set_f32(UNIT_FIELD_MINDAMAGE, (b.min_damage * ratio * pct).max(1.0));
    f.set_f32(UNIT_FIELD_MAXDAMAGE, (b.max_damage * ratio * pct).max(1.0));
    // A creature's ranged attack power and damage the same way, by its
    // ranged auras and its agility's change; a pet has none.
    let mut ranged_ap = 0;
    if model == PowerModel::Creature {
        let ranged_base = b.ranged_attack_power as i32
            + stat_ranged_attack_power(class, level, agi)
            - stat_ranged_attack_power(class, level, b.stats[1]);
        let rap_mods = a.sum(AURA_MOD_RANGED_ATTACK_POWER);
        let rap = ((ranged_base + rap_mods) as f32 * a.product(AURA_MOD_RANGED_ATTACK_POWER_PCT))
            .max(0.0);
        ranged_ap = rap as u32;
        f.set_u32(UNIT_FIELD_RANGED_ATTACK_POWER, ranged_base.max(0) as u32);
        f.set_u32(UNIT_FIELD_RANGED_ATTACK_POWER_MODS, rap_mods.max(0) as u32);
        if b.max_ranged > 0.0 {
            let ratio = if b.ranged_attack_power > 0 {
                let seed = b.ranged_attack_power as f32;
                (7.0 * seed + 3.0 * rap) / (10.0 * seed)
            } else {
                1.0
            };
            f.set_f32(UNIT_FIELD_MINRANGEDDAMAGE, b.min_ranged * ratio * pct);
            f.set_f32(UNIT_FIELD_MAXRANGEDDAMAGE, b.max_ranged * ratio * pct);
        }
    }
    let haste = period_multiplier(a, &[AURA_MOD_HASTE, AURA_MOD_ATTACKSPEED]);
    let attack = if b.attack_time > 0 {
        b.attack_time
    } else {
        2000
    };
    f.set_u32(
        UNIT_FIELD_BASEATTACKTIME,
        ((attack as f32 * haste) as u32).max(100),
    );
    f.set_u32(
        UNIT_FIELD_BASEATTACKTIME + 1,
        ((attack as f32 * haste) as u32).max(100),
    );
    let cast = period_multiplier(a, &[AURA_MOD_CASTING_SPEED]);
    f.set_f32(UNIT_MOD_CAST_SPEED, cast);
    let level = f.get_u32(UNIT_FIELD_LEVEL);
    c.derived = Derived {
        crit: (5.0 + a.sum(AURA_MOD_CRIT_PERCENT) as f32).max(0.0),
        ranged_crit: (5.0 + a.sum(AURA_MOD_CRIT_PERCENT) as f32).max(0.0),
        dodge: (5.0 + a.sum(AURA_MOD_DODGE_PERCENT) as f32).max(0.0),
        parry: (5.0 + a.sum(AURA_MOD_PARRY_PERCENT) as f32).max(0.0),
        block: (5.0 + a.sum(AURA_MOD_BLOCK_PERCENT) as f32).max(0.0),
        spell_crit: std::array::from_fn(|i| {
            (a.sum(AURA_MOD_SPELL_CRIT_CHANCE)
                + a.sum_mask(AURA_MOD_SPELL_CRIT_CHANCE_SCHOOL, 1 << i))
            .max(0) as f32
        }),
        hit: a.sum(AURA_MOD_HIT_CHANCE) as f32,
        off_hit: a.sum(AURA_MOD_HIT_CHANCE) as f32,
        ranged_hit: a.sum(AURA_MOD_HIT_CHANCE) as f32,
        spell_hit: a.sum(AURA_MOD_SPELL_HIT_CHANCE) as f32,
        block_value: level / 2,
        attack_power: ap as u32,
        ranged_attack_power: ranged_ap,
        spell_power: std::array::from_fn(|i| a.sum_mask(AURA_MOD_DAMAGE_DONE, 1 << i)),
        healing_power: a.sum(AURA_MOD_HEALING_DONE),
        weapon_skill: level * 5,
        off_skill: level * 5,
        ranged_skill: level * 5,
        defense: level * 5,
        pvp_weapon_skill: level * 5,
        pvp_off_skill: level * 5,
        pvp_ranged_skill: level * 5,
        pvp_defense: level * 5,
        dual_wield: false,
    };
}

/// `SPELL_AURA_USE_NORMAL_MOVEMENT_SPEED`: a cap on the speed, in yards a
/// second (the Silithyst carrier's, "I can't drive 55").
const AURA_USE_NORMAL_MOVEMENT_SPEED: u32 = 191;
const RUN_SPEED: f32 = 7.0;
const SWIM_SPEED: f32 = 4.722_222;

/// What the auras make of a base run speed, as mangos's `UpdateSpeed` and
/// Thistle Tea's `MovementStats` have it: the best speed bonus (a
/// mount's, mounted) times the better of the stacking and the
/// non-stacking ones; capped by a normal-speed aura; then the worst slow.
pub fn run_multiplier(a: &Auras, mounted: bool) -> f32 {
    let (main, always, not_stack) = if mounted {
        (
            AURA_MOD_INCREASE_MOUNTED_SPEED,
            AURA_MOD_MOUNTED_SPEED_ALWAYS,
            AURA_MOD_MOUNTED_SPEED_NOT_STACK,
        )
    } else {
        (
            AURA_MOD_INCREASE_SPEED,
            AURA_MOD_SPEED_ALWAYS,
            AURA_MOD_SPEED_NOT_STACK,
        )
    };
    let bonus = a
        .product(always)
        .max((100 + a.max(not_stack).max(0)) as f32 / 100.0);
    let up = bonus * (100 + a.max(main).max(0)) as f32 / 100.0;
    normal_capped(a, up, RUN_SPEED) * slow_multiplier(a)
}

/// The swim speed's: the best swim bonus, capped, then the worst slow.
fn swim_multiplier(a: &Auras) -> f32 {
    let up = (100 + a.max(AURA_MOD_INCREASE_SWIM_SPEED).max(0)) as f32 / 100.0;
    normal_capped(a, up, SWIM_SPEED) * slow_multiplier(a)
}

fn normal_capped(a: &Auras, multiplier: f32, base: f32) -> f32 {
    let limit = a.max(AURA_USE_NORMAL_MOVEMENT_SPEED);
    if limit > 0 {
        multiplier.min(limit as f32 / base)
    } else {
        multiplier
    }
}

/// The worst slow, alone.
fn slow_multiplier(a: &Auras) -> f32 {
    ((100 + a.min(AURA_MOD_DECREASE_SPEED).min(0)) as f32 / 100.0).max(0.0)
}

/// Run speed with the auras' hastes and slows, sent where it changed.
fn speed(map: &mut Map, g: Guid) {
    use crate::game::conn::{Order, SpeedKind};
    use crate::game::map::misc::{
        MSG_MOVE_SET_RUN_BACK_SPEED, MSG_MOVE_SET_RUN_SPEED, MSG_MOVE_SET_SWIM_BACK_SPEED,
        MSG_MOVE_SET_SWIM_SPEED,
    };
    use wow_world_messages::vanilla::*;
    let mounted = map
        .unit_fields(g)
        .is_some_and(|f| f.get_u32(UNIT_FIELD_MOUNTDISPLAYID) != 0);
    // What changed, for those who see the player too (Thistle Tea's
    // `broadcast_speed`).
    let mut told: Vec<(u16, f32)> = Vec::new();
    if let Some(p) = map.players.get_mut(&g) {
        let speed = (RUN_SPEED * p.speed_rate * run_multiplier(&p.auras, mounted)).max(0.5);
        // A ghost's speeds are its own (`death`); one lying dead takes
        // back what its auras took as they go (mangos's `UpdateSpeed` on
        // the auras removed at death, Thistle Tea's death restoring them).
        let ghost = matches!(p.death, crate::game::map::death::Death::Ghost { .. });
        if !ghost && (speed - p.speeds.run).abs() > 0.01 {
            p.speeds.run = speed;
            p.outbound.send(&SMSG_FORCE_RUN_SPEED_CHANGE {
                guid: g,
                move_event: p.outbound.order(Order::Speed {
                    guid: g,
                    kind: SpeedKind::Run,
                    speed,
                }),
                speed,
            });
            told.push((MSG_MOVE_SET_RUN_SPEED, speed));
        }
        // Backwards, only slows count (Thistle Tea's `MovementStats`).
        let back = 4.5 * slow_multiplier(&p.auras);
        if !ghost && (back - p.speeds.run_back).abs() > 0.01 {
            p.speeds.run_back = back;
            p.outbound.send(&SMSG_FORCE_RUN_BACK_SPEED_CHANGE {
                guid: g,
                move_event: p.outbound.order(Order::Speed {
                    guid: g,
                    kind: SpeedKind::RunBack,
                    speed: back,
                }),
                speed: back,
            });
            told.push((MSG_MOVE_SET_RUN_BACK_SPEED, back));
        }
        if !ghost && (2.5 - p.speeds.swim_back).abs() > 0.01 {
            p.speeds.swim_back = 2.5;
            p.outbound.send(&SMSG_FORCE_SWIM_BACK_SPEED_CHANGE {
                guid: g,
                move_event: p.outbound.order(Order::Speed {
                    guid: g,
                    kind: SpeedKind::SwimBack,
                    speed: 2.5,
                }),
                speed: 2.5,
            });
            told.push((MSG_MOVE_SET_SWIM_BACK_SPEED, 2.5));
        }
        // Forward swimming: the strongest swim bonus, and slows (Thistle
        // Tea's swimming speeds, vmangos's `Unit::UpdateSpeed`).
        let swim = SWIM_SPEED * swim_multiplier(&p.auras);
        if !ghost && (swim - p.speeds.swim).abs() > 0.01 {
            p.speeds.swim = swim;
            p.outbound.send(&SMSG_FORCE_SWIM_SPEED_CHANGE {
                guid: g,
                move_event: p.outbound.order(Order::Speed {
                    guid: g,
                    kind: SpeedKind::Swim,
                    speed: swim,
                }),
                speed: swim,
            });
            told.push((MSG_MOVE_SET_SWIM_SPEED, swim));
        }
        for (opcode, speed) in told {
            crate::game::map::misc::speed_to_observers(map, g, opcode, speed);
        }
        return;
    }
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let mult = run_multiplier(&c.auras, mounted);
    let base = if c.base.run_speed > 0.0 {
        c.base.run_speed
    } else {
        RUN_SPEED
    };
    // Wounds slow it, past the auras.
    let speed = (base * mult * c.wounded).max(0.5);
    if (speed - c.speeds.run).abs() > 0.01 {
        let was = std::mem::replace(&mut c.speeds.run, speed);
        // Where it runs, it runs at its new speed (mangos's
        // `propagateSpeedChange`, Thistle Tea's `retime`).
        let (now, spline_id) = (map.now, map.next_spline_id);
        if let Some(c) = map.creature_mut(g)
            && let Some(frame) = c.retime(was, now, spline_id)
        {
            map.next_spline_id = map.next_spline_id.wrapping_add(1);
            map.send_to_observers(g, &frame, false);
        }
        // A unit a player's client moves (Mind Control, Eyes of the
        // Beast) hears it as a player does, and those who see it once that
        // client has it; the rest by the spline's speed, as mangos-classic
        // tells them (Thistle Tea tells only the possessed). Its
        // controller is its charmer, summoner, creator or owner, not any
        // player on the map.
        match crate::game::map::movement::controller(map, g) {
            Some(p) => {
                p.outbound.send(&SMSG_FORCE_RUN_SPEED_CHANGE {
                    guid: g,
                    move_event: p.outbound.order(Order::Speed {
                        guid: g,
                        kind: SpeedKind::Run,
                        speed,
                    }),
                    speed,
                });
            }
            None => {
                if let Some(frame) =
                    crate::game::conn::encode(&SMSG_SPLINE_SET_RUN_SPEED { guid: g, speed })
                {
                    map.send_to_observers(g, &frame, false);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crit_and_dodge_follow_the_class_rates() {
        // A level 60 rogue with 300 agility: 300 / 29 crit.
        assert!((agility_rate(ROGUE, 60, false) - 29.0).abs() < 0.01);
        assert!((agility_rate(WARRIOR, 1, false) - 3.9).abs() < 0.01);
        assert!((spell_crit_chance(MAGE, 60, 250) - (3.70 + 250.0 / (14.77 + 39.0))).abs() < 0.01);
        assert_eq!(melee_attack_power(WARRIOR, 60, 300, 100), 760);
        assert_eq!(ranged_attack_power(HUNTER, 60, 300), 710);
        assert!((speed_factor(25) - 0.8).abs() < 0.001);
        assert!((speed_factor(-50) - 1.5).abs() < 0.001);
    }
}
