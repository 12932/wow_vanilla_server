//! What a spell does when it lands: whether it hits (vmangos's magic hit by
//! level, binary resists, the special attack table for melee and ranged
//! abilities), whether it crits, what spell power adds (each effect's
//! coefficient, or vmangos's cast-time formula), the modifiers of damage and
//! healing done and taken, partial resists, absorbs, and the logs.

use rand::RngExt;
use wow_world_messages::Guid;

use crate::game::data::items::Body;
use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat;
use crate::game::map::procs;

use super::mods::{self, Mods};

pub const SMSG_SPELLLOGMISS: u16 = 0x24B;
pub const SMSG_SPELLNONMELEEDAMAGELOG: u16 = 0x250;
pub const SMSG_SPELLHEALLOG: u16 = 0x150;
/// `SpellHitType`: a crit.
const SPELL_HIT_TYPE_CRIT: u32 = 0x2;

/// How a spell went for one target (`SpellMissInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hit {
    #[default]
    Hit,
    Miss,
    Resist,
    Dodge,
    Parry,
    /// Hit, but blocked: the damage less the block value.
    Block,
    Evade,
    Immune,
    Deflect,
    Absorb,
    Reflect,
}

impl Hit {
    pub fn lands(self) -> bool {
        matches!(self, Hit::Hit | Hit::Block)
    }

    fn wire(self) -> u8 {
        match self {
            Hit::Hit => 0,
            Hit::Miss => 1,
            Hit::Resist => 2,
            Hit::Dodge => 3,
            Hit::Parry => 4,
            Hit::Block => 5,
            Hit::Evade => 6,
            Hit::Immune => 7,
            Hit::Deflect => 9,
            Hit::Absorb => 10,
            Hit::Reflect => 11,
        }
    }

    pub fn proc_ex(self) -> u32 {
        match self {
            Hit::Hit => procs::EX_NORMAL_HIT,
            Hit::Miss => procs::EX_MISS,
            Hit::Resist => procs::EX_RESIST,
            Hit::Dodge => procs::EX_DODGE,
            Hit::Parry => procs::EX_PARRY,
            Hit::Block => procs::EX_BLOCK | procs::EX_NORMAL_HIT,
            Hit::Evade => procs::EX_EVADE,
            Hit::Immune => procs::EX_IMMUNE,
            Hit::Deflect => procs::EX_DEFLECT,
            Hit::Absorb => procs::EX_ABSORB,
            Hit::Reflect => procs::EX_REFLECT,
        }
    }
}

/// What a spell knows of its hit on one target as its effects run.
#[derive(Debug, Clone, Default)]
pub struct Landing {
    pub hit: Hit,
    pub crit: bool,
    /// The damage or healing share a chain's later targets get.
    pub chain: f32,
    pub combo: u8,
    pub mods: Mods,
    /// The spell was reflected: the caster is its target.
    pub reflected: bool,
}

/// Whether spell outcomes are rolled (production), or fixed: always a hit,
/// never a crit or a partial resist below a certain chance (tests).
pub fn rolling(map: &Map) -> bool {
    map.spells.rolls.unwrap_or(map.realm.config.spell_rolls)
}

/// A roll from 0 to 9,999, hundredths of a percent.
fn roll(map: &Map) -> i32 {
    if rolling(map) {
        rand::rng().random_range(0..10_000)
    } else {
        // Fixed: anything less than certain fails, and certainty holds.
        9_999
    }
}

/// Whether a `chance` (in percent) comes up.
pub fn chance(map: &Map, chance: f32) -> bool {
    if chance >= 100.0 {
        return true;
    }
    if chance <= 0.0 {
        return false;
    }
    roll(map) < (chance * 100.0) as i32
}

fn derived(map: &Map, g: Guid) -> crate::game::map::derive::Derived {
    if let Some(p) = map.players.get(&g) {
        return p.derived;
    }
    map.creature(g).map(|c| c.derived).unwrap_or_default()
}

fn level(map: &Map, g: Guid) -> u32 {
    map.unit_view(g).map(|u| u.level).unwrap_or(1).max(1)
}

/// The school's index: 0 physical to 6 arcane.
pub fn school_index(s: &SpellTemplate) -> usize {
    s.school.min(6) as usize
}

/// A spell all or nothing: magic with a control or debuff effect, which a
/// resistance resists whole or not at all.
pub fn binary(s: &SpellTemplate) -> bool {
    if s.dmg_class != 1 || s.school == 0 {
        return false;
    }
    matches!(s.id, 26143 | 26478)
        || s.effects.iter().any(|e| {
            matches!(e.effect, 68 | 98)
                || (e.effect == 6
                    && matches!(
                        e.aura,
                        AURA_MOD_DECREASE_SPEED
                            | AURA_MOD_FEAR
                            | AURA_MOD_STUN
                            | AURA_MOD_PACIFY
                            | AURA_MOD_ROOT
                            | AURA_MOD_SILENCE
                            | AURA_MOD_DISARM
                            | AURA_MOD_RESISTANCE
                            | AURA_MOD_DAMAGE_TAKEN
                    ))
        })
}

/// A spell that hits an area.
pub fn area(s: &SpellTemplate) -> bool {
    s.effects.iter().any(|e| {
        matches!(
            e.target_a,
            2 | 15 | 16 | 20 | 22 | 24 | 28 | 30 | 31 | 33 | 34 | 54
        ) || matches!(e.target_b, 15 | 16 | 24 | 28 | 30 | 31 | 54)
    })
}

/// Whether a spell is a melee or ranged ability: rolled on the attack table,
/// crits for double.
pub fn physical(s: &SpellTemplate) -> bool {
    s.melee_or_ranged()
}

/// The target's resistance to `school`, less what the caster's auras and
/// gear take off it.
pub fn resistance(map: &Map, caster: Guid, target: Guid, school: usize) -> i32 {
    let base = map
        .unit_fields(target)
        .map(|f| f.get_u32(UNIT_FIELD_RESISTANCES + school as u16) as i32)
        .unwrap_or(0);
    let pen = auras::auras(map, caster)
        .map_or(0, |a| a.sum_mask(AURA_MOD_TARGET_RESISTANCE, 1 << school));
    (base + pen).max(0)
}

/// vmangos's chance (0 to 0.75) that a resistance resists, against a
/// caster of `caster_level`; a creature resists more the higher it is.
pub fn resist_chance(
    resistance: i32,
    caster_level: u32,
    target_creature: bool,
    level_diff: i32,
) -> f32 {
    let caster_level = caster_level.max(1) as f32;
    let mut r = resistance.max(0) as f32;
    if target_creature && level_diff > 0 {
        r += (8.0 * level_diff as f32 * caster_level / 63.0).trunc();
    }
    (r * 0.15 / caster_level).clamp(0.0, 0.75)
}

/// The share of damage resisted, from vmangos's buckets: 0, 25, 50 or 75%.
pub fn resist_fraction(chance_pct: f32, roll: i32) -> f32 {
    const TABLE: [(f32, f32, f32, f32, f32); 31] = [
        (0.0, 0.0, 0.0, 0.0, 0.0),
        (3.0, 0.0, 0.0, 2.0, 6.0),
        (5.0, 0.0, 1.0, 4.0, 12.0),
        (8.0, 0.0, 1.0, 5.0, 18.0),
        (10.0, 0.0, 1.0, 7.0, 23.0),
        (13.0, 0.0, 2.0, 9.0, 28.0),
        (15.0, 0.0, 2.0, 11.0, 33.0),
        (18.0, 0.0, 2.0, 13.0, 37.0),
        (20.0, 0.0, 3.0, 15.0, 41.0),
        (23.0, 1.0, 3.0, 17.0, 46.0),
        (25.0, 1.0, 4.0, 19.0, 47.0),
        (28.0, 1.0, 5.0, 21.0, 48.0),
        (30.0, 1.0, 6.0, 24.0, 49.0),
        (33.0, 1.0, 8.0, 28.0, 47.0),
        (35.0, 1.0, 9.0, 33.0, 43.0),
        (38.0, 1.0, 11.0, 37.0, 39.0),
        (40.0, 1.0, 13.0, 41.0, 35.0),
        (43.0, 1.0, 16.0, 45.0, 30.0),
        (45.0, 1.0, 18.0, 48.0, 26.0),
        (48.0, 2.0, 20.0, 48.0, 24.0),
        (50.0, 4.0, 23.0, 48.0, 21.0),
        (53.0, 5.0, 25.0, 47.0, 19.0),
        (55.0, 7.0, 28.0, 45.0, 17.0),
        (58.0, 9.0, 31.0, 43.0, 16.0),
        (60.0, 11.0, 34.0, 40.0, 14.0),
        (62.0, 13.0, 37.0, 37.0, 12.0),
        (65.0, 15.0, 41.0, 33.0, 10.0),
        (68.0, 18.0, 44.0, 29.0, 8.0),
        (70.0, 20.0, 48.0, 25.0, 7.0),
        (73.0, 23.0, 51.0, 20.0, 5.0),
        (75.0, 25.0, 55.0, 16.0, 3.0),
    ];
    let c = chance_pct.clamp(0.0, 75.0);
    let i = TABLE.iter().position(|t| t.0 >= c).unwrap_or(30).max(1);
    let (p, n) = (TABLE[i - 1], TABLE[i]);
    let k = (c - p.0) / (n.0 - p.0).max(1.0);
    let lerp = |a: f32, b: f32| a + (b - a) * k;
    let (r100, r75, r50, r25) = (
        lerp(p.1, n.1),
        lerp(p.2, n.2),
        lerp(p.3, n.3),
        lerp(p.4, n.4),
    );
    let roll = roll as f32 / 100.0;
    if roll < r100 + r75 {
        0.75
    } else if roll < r100 + r75 + r50 {
        0.5
    } else if roll < r100 + r75 + r50 + r25 {
        0.25
    } else {
        0.0
    }
}

/// vmangos's magic hit chance in hundredths of a percent: 96% at even
/// level, less 1% a level to two above, then 7 (players) or 11 (creatures)
/// a level, 22% at worst; with the caster's hit, less the target's
/// mechanic resistance, and less a binary spell's resist chance.
pub fn magic_hit_chance(
    caster_level: u32,
    target_level: u32,
    target_player: bool,
    hit_bonus: f32,
    mechanic_resistance: f32,
    binary_resist: f32,
) -> i32 {
    let diff = target_level as i32 - caster_level as i32;
    let per = if target_player { 7 } else { 11 };
    let level = if diff < 3 {
        96 - diff
    } else {
        94 - (diff - 2) * per
    }
    .max(22);
    let v = ((level as f32 * 100.0 + hit_bonus * 100.0 - mechanic_resistance * 100.0)
        * (1.0 - binary_resist)) as i32;
    v.clamp(100, 9_900)
}

/// Whether `s` from `caster` hits `target`, and if not, why.
pub fn roll_hit(map: &Map, caster: Guid, target: Guid, s: &SpellTemplate, m: &Mods) -> Hit {
    if target == caster || auras::positive(s) {
        return Hit::Hit;
    }
    if map.creature(target).is_some_and(|c| c.evading) {
        return Hit::Evade;
    }
    let Some(a) = auras::auras(map, target) else {
        return Hit::Hit;
    };
    if !s.attr(0, sd::ATTR_NO_IMMUNITIES)
        && !s.attr(2, sd::ATTR_EX2_NO_SCHOOL_IMMUNITIES)
        && a.school_immune(s.school_mask())
    {
        return Hit::Immune;
    }
    let all_mechanics_immune = {
        let mechanics: Vec<u32> = std::iter::once(s.mechanic)
            .chain(
                s.effects
                    .iter()
                    .filter(|e| e.effect != 0)
                    .map(|e| e.mechanic),
            )
            .filter(|m| *m != 0)
            .collect();
        !mechanics.is_empty()
            && s.effects.iter().filter(|e| e.effect != 0).all(|e| {
                a.mechanic_immune(if e.mechanic != 0 {
                    e.mechanic
                } else {
                    s.mechanic
                })
            })
    };
    if all_mechanics_immune && !s.attr(0, sd::ATTR_NO_IMMUNITIES) {
        return Hit::Immune;
    }
    if s.attr(3, sd::ATTR_EX3_ALWAYS_HIT) {
        return Hit::Hit;
    }
    if physical(s) {
        return melee_roll(map, caster, target, s, m).0;
    }
    // Reflection of a single-target magic spell.
    if !area(s) && s.dmg_class == 1 && !s.attr(1, sd::ATTR_EX_NO_REFLECTION) {
        let reflect = a.sum(AURA_REFLECT_SPELLS)
            + a.sum_mask(AURA_REFLECT_SPELLS_SCHOOL, s.school_mask())
            + ward_reflect(map, target, s);
        if reflect > 0 && chance(map, reflect as f32) {
            return Hit::Reflect;
        }
    }
    let caster_level = level(map, caster);
    let target_level = level(map, target);
    let d = derived(map, caster);
    let hit_bonus = d.spell_hit + m.flat[mods::OP_RESIST_MISS_CHANCE] as f32;
    let mechanic = std::iter::once(s.mechanic)
        .chain(s.effects.iter().map(|e| e.mechanic))
        .filter(|m| *m != 0)
        .map(|mech| a.sum_misc(AURA_MOD_MECHANIC_RESISTANCE, mech as i32))
        .max()
        .unwrap_or(0) as f32;
    let binary_resist = if binary(s) {
        resist_chance(
            resistance(map, caster, target, school_index(s)),
            caster_level,
            false,
            0,
        )
    } else {
        0.0
    };
    let aoe = if area(s) {
        a.sum(AURA_MOD_AOE_AVOIDANCE) as f32
    } else {
        0.0
    } + a.sum_mask(AURA_MOD_RESIST_CHANCE, s.school_mask()) as f32;
    let chance = magic_hit_chance(
        caster_level,
        target_level,
        guid::is_player(target),
        hit_bonus - aoe,
        mechanic,
        binary_resist,
    );
    if roll_strict(map, chance) {
        Hit::Hit
    } else {
        Hit::Resist
    }
}

/// Improved Fire Ward and Frost Warding: a ward up against its school may
/// throw the spell back.
fn ward_reflect(map: &Map, target: Guid, s: &SpellTemplate) -> i32 {
    let data = &map.realm.data;
    let Some(a) = auras::auras(map, target) else {
        return 0;
    };
    // The ward on the target: Fire Ward's family flag 0x8, Frost Ward's
    // 0x100.
    let ward = a.holders.iter().find_map(|h| {
        let t = data.spells.get(h.spell)?;
        let flags = t.family_flags as u32 & 0x108;
        let guards = h
            .effects
            .iter()
            .any(|x| x.kind == AURA_SCHOOL_ABSORB && (x.misc as u32) & s.school_mask() != 0);
        (t.family_name == 3 && flags != 0 && guards).then_some(flags)
    });
    let Some(ward) = ward else {
        return 0;
    };
    let talents: [(u32, u32); 4] = [(11094, 0x8), (13043, 0x8), (11189, 0x100), (28332, 0x100)];
    talents
        .iter()
        .filter(|(_, mask)| mask & ward != 0)
        .filter_map(|(id, _)| {
            a.holders.iter().find(|h| h.spell == *id).and_then(|h| {
                h.effects
                    .iter()
                    .find(|x| x.kind == AURA_DUMMY)
                    .map(|x| x.amount)
            })
        })
        .max()
        .unwrap_or(0)
}

/// A hit roll: in the fixed rolls of tests, anything above 1% hits.
fn roll_strict(map: &Map, chance_bp: i32) -> bool {
    if !rolling(map) {
        return chance_bp > 100;
    }
    rand::rng().random_range(0..10_000) < chance_bp
}

/// A melee or ranged ability's roll on the combat table (Thistle Tea's
/// `attack_table.ex`, as combat rolls a special swing), with the spell's
/// crit modifiers and crit against the target's type: the outcome, and
/// whether it crits.
pub fn melee_roll(
    map: &Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    m: &Mods,
) -> (Hit, bool) {
    let hand = if s.dmg_class == 3 {
        combat::Hand::Ranged
    } else if s.attr(3, sd::ATTR_EX3_REQ_OFFHAND) {
        combat::Hand::Off
    } else {
        combat::Hand::Main
    };
    let Some(mut r) = combat::roll_for(map, caster, target, hand) else {
        return (Hit::Miss, false);
    };
    r.crit += m.flat[mods::OP_CRITICAL_CHANCE] as f32 + crit_versus(map, caster, target);
    r.can_crit = !s.attr(2, sd::ATTR_EX2_CANT_CRIT);
    if s.attr(0, 0x20_0000) {
        r.avoidance_disabled = true;
    }
    match combat::roll_special(&r, roll(map), roll(map)) {
        combat::Outcome::Miss => (Hit::Miss, false),
        combat::Outcome::Dodge => (Hit::Dodge, false),
        combat::Outcome::Parry => (Hit::Parry, false),
        combat::Outcome::Block => (Hit::Block, false),
        combat::Outcome::Crit => (Hit::Hit, true),
        _ => (Hit::Hit, false),
    }
}

/// Whether a damage or heal effect of `s` crits.
pub fn roll_crit(map: &Map, caster: Guid, target: Guid, s: &SpellTemplate, m: &Mods) -> bool {
    if s.attr(2, sd::ATTR_EX2_CANT_CRIT) {
        return false;
    }
    let d = derived(map, caster);
    let mut c = if physical(s) {
        if s.dmg_class == 3 {
            d.ranged_crit
        } else {
            d.crit
        }
    } else {
        d.spell_crit[school_index(s)]
    } + m.flat[mods::OP_CRITICAL_CHANCE] as f32;
    // Shatter: more against the frozen.
    if !physical(s) && frozen(map, target) {
        c += shatter(map, caster, s);
    }
    c += crit_versus(map, caster, target);
    chance(map, c)
}

/// More crit against a creature type (Monster Slaying, Humanoid Slaying).
pub fn crit_versus(map: &Map, caster: Guid, target: Guid) -> f32 {
    let creature_type = map
        .creature(target)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .map(|t| t.creature_type)
        .unwrap_or(7);
    if creature_type == 0 {
        return 0.0;
    }
    let bit = 1u32 << (creature_type - 1).min(31);
    auras::auras(map, caster).map_or(0.0, |a| {
        a.sum_mask(AURA_MOD_CRIT_PERCENT_VERSUS, bit) as f32
    })
}

/// Whether a unit is held by a frost spell (rooted or stunned by frost).
pub fn frozen(map: &Map, g: Guid) -> bool {
    auras::auras(map, g).is_some_and(|a| {
        a.holders
            .iter()
            .any(|h| h.school & 0x10 != 0 && (h.has(AURA_MOD_ROOT) || h.has(AURA_MOD_STUN)))
    })
}

/// Shatter's crit chance against the frozen, from the caster's override
/// class scripts 849 to 913.
fn shatter(map: &Map, caster: Guid, s: &SpellTemplate) -> f32 {
    let data = &map.realm.data;
    auras::auras(map, caster).map_or(0.0, |a| {
        a.effects()
            .filter(|(h, x)| {
                x.kind == AURA_OVERRIDE_CLASS_SCRIPTS
                    && data
                        .spells
                        .get(h.spell)
                        .is_some_and(|t| t.family_name == s.family_name)
            })
            .map(|(_, x)| match x.misc {
                849 => 10.0,
                910 => 20.0,
                911 => 30.0,
                912 => 40.0,
                913 => 50.0,
                _ => 0.0,
            })
            .sum()
    })
}

/// Direct, or over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Direct,
    Dot,
}

/// The bonus coefficient of effect `index`: the template's, or vmangos's
/// formula from the cast time (DoTs from their duration), halved for areas
/// and leeches, less for control effects, and less below level 20.
pub fn coefficient(map: &Map, s: &SpellTemplate, index: usize, kind: Kind) -> f32 {
    let e = &s.effects[index];
    if e.bonus_coefficient >= 0.0 {
        return e.bonus_coefficient;
    }
    let data = &map.realm.data;
    let cast = data.spells.cast_time(s);
    let duration = data.spells.duration(s, 60).unwrap_or(0);
    let periodic = s.effects.iter().any(|x| {
        matches!(
            x.aura,
            AURA_PERIODIC_DAMAGE | AURA_PERIODIC_HEAL | AURA_PERIODIC_LEECH
        )
    });
    let direct = s
        .effects
        .iter()
        .any(|x| matches!(x.effect, 2 | 8 | 9 | 10 | 62));
    let channeled = s.channeled();
    let clamp = |ms: u32| (ms as f32).clamp(1500.0, 7000.0);
    let mut time = if channeled {
        clamp(duration)
    } else {
        clamp(cast)
    };
    if kind == Kind::Dot && !channeled {
        time = 3500.0;
    }
    let over_time = if periodic { duration as f32 } else { 0.0 };
    if over_time > 0.0 && time > 0.0 && direct {
        let original = clamp(cast);
        let share = over_time / 15000.0 / (over_time / 15000.0 + original / 3500.0);
        time = match kind {
            Kind::Dot => time * share,
            Kind::Direct if share < 1.0 => time * (1.0 - share),
            Kind::Direct => 0.0,
        };
    }
    if area(s) {
        time /= 2.0;
    }
    if s.effects
        .iter()
        .any(|x| x.effect == 9 || x.aura == AURA_PERIODIC_LEECH)
    {
        time /= 2.0;
    }
    let extra = s.effects.iter().fold(0, |n, x| match x.aura {
        AURA_DUMMY | AURA_MOD_DECREASE_SPEED => n + 1,
        AURA_MOD_CONFUSE | AURA_MOD_STUN | AURA_MOD_ROOT => n + 2,
        _ => n,
    });
    time *= 0.95f32.powi(extra);
    let mut c = time / 3500.0;
    if kind == Kind::Dot {
        let base = if channeled {
            1.0
        } else {
            duration as f32 / 15000.0
        };
        let ticks = s
            .effects
            .iter()
            .filter(|x| x.amplitude > 0)
            .map(|x| duration / x.amplitude)
            .max()
            .unwrap_or(0);
        let factor = if ticks > 0 { base / ticks as f32 } else { base };
        c = time / 3500.0 * factor;
    }
    c * level_penalty(s)
}

/// Spells learned below level 20 get less of the caster's power.
pub fn level_penalty(s: &SpellTemplate) -> f32 {
    if (1..=20).contains(&s.spell_level) {
        1.0 - (20 - s.spell_level) as f32 * 0.0375
    } else {
        1.0
    }
}

/// The caster's spell power for `s`, or healing power, including the
/// creature-type bonuses against `target`.
pub fn power(map: &Map, caster: Guid, target: Guid, s: &SpellTemplate, heal: bool) -> f32 {
    let d = derived(map, caster);
    if heal {
        let spirit = map
            .unit_fields(caster)
            .map(|f| f.get_u32(UNIT_FIELD_STAT0 + 4) as i32)
            .unwrap_or(0);
        let a = auras::auras(map, caster);
        let done = a.map_or(0, |a| a.sum(AURA_MOD_HEALING_DONE));
        let from_spirit = a.map_or(0, |a| {
            spirit * a.sum(AURA_MOD_SPELL_HEALING_OF_STAT_PERCENT) / 100
        });
        return (done + from_spirit.max(0)) as f32;
    }
    let mut p = d.spell_power[school_index(s)] as f32;
    let creature_type = map
        .creature(target)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .map(|t| t.creature_type)
        .unwrap_or(7);
    if creature_type > 0
        && let Some(a) = auras::auras(map, caster)
    {
        p += a
            .effects()
            .filter(|(_, x)| {
                x.kind == AURA_MOD_DAMAGE_DONE_CREATURE
                    && (x.misc as u32) & (1 << (creature_type - 1).min(31)) != 0
            })
            .map(|(h, x)| (x.amount * h.stacks.max(1) as i32) as f32)
            .sum::<f32>();
    }
    p
}

/// What spell power (or healing power) adds to effect `index`.
pub fn bonus(
    map: &Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    kind: Kind,
    m: &Mods,
) -> f32 {
    if physical(s) || s.attr(3, sd::ATTR_EX3_IGNORE_CASTER_MODIFIERS) {
        return 0.0;
    }
    let heal = matches!(s.effects[index].effect, 10) || s.effects[index].aura == AURA_PERIODIC_HEAL;
    let p = power(map, caster, target, s, heal);
    if p <= 0.0 {
        return 0.0;
    }
    let c = coefficient(map, s, index, kind) * (100 + m.pct[mods::OP_SPELL_BONUS_DAMAGE]) as f32
        / 100.0;
    p * c + m.flat[mods::OP_SPELL_BONUS_DAMAGE] as f32
}

/// The caster's multiplier on damage of `s`'s school done to `target`.
pub fn done_multiplier(map: &Map, caster: Guid, target: Guid, s: &SpellTemplate) -> f32 {
    let Some(a) = auras::auras(map, caster) else {
        return 1.0;
    };
    // Physical damage's percent is in the weapon's damage already.
    let mut m = if physical(s) && s.school == 0 {
        1.0
    } else {
        a.product_mask(AURA_MOD_DAMAGE_PERCENT_DONE, s.school_mask())
    };
    let creature_type = map
        .creature(target)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .map(|t| t.creature_type)
        .unwrap_or(7);
    if creature_type > 0 {
        let bit = 1 << (creature_type - 1).min(31);
        m *= a.product_mask(AURA_MOD_DAMAGE_DONE_VERSUS, bit);
    }
    m
}

/// Damage `target` takes of `s`: its flat and percent modifiers.
pub fn taken(map: &Map, target: Guid, s: &SpellTemplate, damage: f32) -> f32 {
    let Some(a) = auras::auras(map, target) else {
        return damage;
    };
    let mask = s.school_mask();
    let mut d = damage + a.sum_mask(AURA_MOD_DAMAGE_TAKEN, mask) as f32;
    d *= a.product_mask(AURA_MOD_DAMAGE_PERCENT_TAKEN, mask);
    match s.dmg_class {
        2 => {
            d += a.sum_mask(AURA_MOD_MELEE_DAMAGE_TAKEN, mask) as f32;
            d *= a.product_mask(AURA_MOD_MELEE_DAMAGE_TAKEN_PCT, mask);
        }
        3 => {
            d += a.sum_mask(AURA_MOD_RANGED_DAMAGE_TAKEN, mask) as f32;
            d *= a.product_mask(AURA_MOD_RANGED_DAMAGE_TAKEN_PCT, mask);
        }
        _ => {}
    }
    d.max(0.0)
}

/// What an effect's damage comes to: bonus, done and taken modifiers, the
/// crit, armor for physical damage; then the partial resist.
pub struct Damage {
    pub amount: u32,
    pub resisted: u32,
    pub blocked: u32,
}

/// Work out direct damage of `base` from `s` to `target`.
pub fn damage(
    map: &Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    base: f32,
    l: &Landing,
) -> Damage {
    let mut d = base + bonus(map, caster, target, s, index, Kind::Direct, &l.mods);
    d *= if l.chain > 0.0 { l.chain } else { 1.0 };
    d *= done_multiplier(map, caster, target, s);
    d = l.mods.apply(mods::OP_DAMAGE, d);
    if s.school == 0 {
        let armor = resistance(map, caster, target, 0);
        d *= 1.0 - combat::armor_reduction(armor as u32, level(map, caster));
    }
    d = taken(map, target, s, d);
    if l.crit {
        let pct = l.mods.pct[mods::OP_CRIT_DAMAGE_BONUS] as f32 / 100.0;
        let bonus = if physical(s) {
            let melee =
                auras::auras(map, caster).map_or(0, |a| a.sum(AURA_MOD_CRIT_DAMAGE_BONUS_MELEE));
            d * (1.0 + pct) * (100 + melee) as f32 / 100.0
        } else {
            d * 0.5 * (1.0 + pct)
        };
        d += bonus;
    }
    let mut blocked = 0;
    if l.hit == Hit::Block {
        blocked = derived(map, target).block_value.min(d as u32);
        d -= blocked as f32;
    }
    let amount = d.max(0.0) as u32;
    let resisted = partial_resist(map, caster, target, s, amount, false);
    Damage {
        amount: amount - resisted,
        resisted,
        blocked,
    }
}

/// Damage of a magic school resisted in part: none for physical or binary
/// spells, a tenth as often for a tick.
pub fn partial_resist(
    map: &Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    damage: u32,
    tick: bool,
) -> u32 {
    if damage == 0 || s.school == 0 || binary(s) || !rolling(map) {
        return 0;
    }
    let caster_level = level(map, caster);
    let diff = level(map, target) as i32 - caster_level as i32;
    let r = resistance(map, caster, target, school_index(s));
    let mut c = resist_chance(r, caster_level, !guid::is_player(target), diff) * 100.0;
    if tick {
        c *= 0.1;
    }
    let roll = rand::rng().random_range(0..10_000);
    (damage as f32 * resist_fraction(c, roll)) as u32
}

/// Healing of `base` from `s` to `target`: the healing bonus, done and
/// taken modifiers, and the crit.
pub fn healing(
    map: &Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    base: f32,
    l: &Landing,
) -> u32 {
    let mut h = base + bonus(map, caster, target, s, index, Kind::Direct, &l.mods);
    h *= if l.chain > 0.0 { l.chain } else { 1.0 };
    if let Some(a) = auras::auras(map, caster) {
        h *= a.product(AURA_MOD_HEALING_DONE_PERCENT);
    }
    h = l.mods.apply(mods::OP_DAMAGE, h);
    // Healing Way: more from each Healing Wave on the target.
    if s.family_name == 11
        && s.family_flags & 0x40 != 0
        && let Some(a) = auras::auras(map, target)
    {
        let pct: i32 = a
            .holders
            .iter()
            .filter(|x| x.spell == 29203)
            .flat_map(|x| {
                x.effects
                    .iter()
                    .map(move |e| e.amount * x.stacks.max(1) as i32)
            })
            .sum();
        h *= (100 + pct) as f32 / 100.0;
    }
    h = healing_taken(map, target, h);
    if l.crit {
        h *= 1.5;
    }
    h.max(0.0) as u32
}

/// Healing `target` takes: its flat and percent modifiers.
pub fn healing_taken(map: &Map, target: Guid, amount: f32) -> f32 {
    match auras::auras(map, target) {
        Some(a) => {
            ((amount + a.sum(AURA_MOD_HEALING) as f32) * a.product(AURA_MOD_HEALING_PCT)).max(0.0)
        }
        None => amount,
    }
}

/// `SMSG_SPELLLOGMISS`: `s` missed `target`, and how, as Thistle Tea
/// writes it for the 1.12 client.
pub fn send_miss(map: &Map, caster: Guid, target: Guid, spell: u32, hit: Hit) {
    let mut b = Body::default();
    b.u32(spell)
        .u64(caster.guid())
        .u8(0)
        .u32(1)
        .u64(target.guid())
        .u8(hit.wire());
    let frame = b.frame(SMSG_SPELLLOGMISS);
    map.send_to_observers(caster, &frame, true);
    if let Some(p) = map.players.get(&target)
        && !p.known.contains(&caster)
    {
        p.outbound.send_frame(frame);
    }
}

/// `SMSG_SPELLNONMELEEDAMAGELOG`, in 1.12's layout.
#[allow(clippy::too_many_arguments)]
pub fn damage_log(
    caster: Guid,
    target: Guid,
    spell: u32,
    school: u32,
    damage: u32,
    absorbed: u32,
    resisted: u32,
    blocked: u32,
    crit: bool,
) -> std::sync::Arc<[u8]> {
    let mut b = Body::default();
    crate::game::update::write_packed_guid(&mut b.0, target);
    crate::game::update::write_packed_guid(&mut b.0, caster);
    b.u32(spell)
        .u32(damage)
        .u8(school.min(6) as u8)
        .u32(absorbed)
        .u32(resisted)
        .u8(0)
        .u8(0)
        .u32(blocked)
        .u32(if crit { SPELL_HIT_TYPE_CRIT } else { 0 })
        .u8(0);
    b.frame(SMSG_SPELLNONMELEEDAMAGELOG)
}

/// Deal `d` of `s` to `target`: absorbs, the log, pushback, the damage,
/// the procs and the threat.
pub fn deal(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate, d: Damage, l: &Landing) {
    let amount = split(map, caster, target, s, d.amount);
    let (left, absorbed) = auras::absorb(map, target, amount, s.school_mask());
    let frame = damage_log(
        caster, target, s.id, s.school, left, absorbed, d.resisted, d.blocked, l.crit,
    );
    map.send_to_observers(caster, &frame, true);
    if let Some(p) = map.players.get(&target)
        && !p.known.contains(&caster)
    {
        p.outbound.send_frame(frame);
    }
    super::pushback(map, target);
    if left > 0 {
        auras::interrupt(map, target, INTERRUPT_DAMAGE | INTERRUPT_DIRECT_DAMAGE);
    }
    let mut ex = l.hit.proc_ex();
    if l.crit {
        ex |= procs::EX_CRITICAL_HIT;
    }
    if absorbed > 0 {
        ex |= procs::EX_ABSORB;
    }
    let (done, taken_flag) = proc_flags(s);
    procs::trigger(
        map,
        procs::Event {
            actor: caster,
            target,
            actor_flags: done,
            target_flags: taken_flag,
            ex,
            spell: Some(s.id),
            damage: left,
            school: s.school_mask(),
            offhand: false,
            ranged: s.dmg_class == 3,
            extra_attack: false,
        },
    );
    let items = death_items(map, target, left);
    if left > 0 {
        combat::deal_damage(map, caster, target, left);
    }
    give_death_items(map, target, items);
    super::threat(map, caster, target, s, left as f32, &l.mods);
}

/// Blessing of Sacrifice and its kind: a share of the damage (or a flat
/// amount of each hit) goes to the aura's caster instead.
pub fn split(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate, damage: u32) -> u32 {
    let shares: Vec<(Guid, u32)> = auras::auras(map, target)
        .map(|a| {
            a.effects()
                .filter(|(h, x)| {
                    matches!(x.kind, AURA_SPLIT_DAMAGE_PCT | AURA_SPLIT_DAMAGE_FLAT)
                        && h.caster != target
                        && (x.misc as u32) & s.school_mask() != 0
                })
                .map(|(h, x)| {
                    let v = if x.kind == AURA_SPLIT_DAMAGE_PCT {
                        damage * x.amount.max(0) as u32 / 100
                    } else {
                        (x.amount.max(0) as u32).min(damage)
                    };
                    (h.caster, v)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut left = damage;
    for (to, share) in shares {
        let share = share.min(left);
        if share == 0 || map.unit_view(to).is_none_or(|u| !u.alive) {
            continue;
        }
        left -= share;
        let frame = damage_log(caster, to, s.id, s.school, share, 0, 0, 0, false);
        map.send_to_observers(to, &frame, true);
        combat::deal_damage(map, caster, to, share);
    }
    left
}

/// The items a unit's death under `AURA_CHANNEL_DEATH_ITEM` leaves its
/// casters (Drain Soul's and Shadowburn's soul shards), if `damage` kills
/// it: only for a death that gives experience or honor.
pub fn death_items(map: &Map, target: Guid, damage: u32) -> Vec<(Guid, u32)> {
    let health = map
        .unit_fields(target)
        .map(|f| f.get_u32(UNIT_FIELD_HEALTH))
        .unwrap_or(0);
    if damage < health || health == 0 {
        return Vec::new();
    }
    let Some(a) = auras::auras(map, target) else {
        return Vec::new();
    };
    a.effects()
        .filter(|(h, x)| {
            x.kind == AURA_CHANNEL_DEATH_ITEM && x.mask != 0 && guid::is_player(h.caster)
        })
        .filter(|(h, _)| {
            let caster_level = level(map, h.caster);
            let target_level = level(map, target);
            guid::is_player(target) || target_level > combat::gray_level(caster_level)
        })
        .map(|(h, x)| (h.caster, x.mask))
        .collect()
}

pub fn give_death_items(map: &mut Map, target: Guid, items: Vec<(Guid, u32)>) {
    if items.is_empty() || map.unit_view(target).is_some_and(|u| u.alive) {
        return;
    }
    for (caster, item) in items {
        let _ = crate::game::map::inventory::add_item(map, caster, item, 1, true);
    }
}

/// The proc flags of a spell's hit, for its caster and its target.
pub fn proc_flags(s: &SpellTemplate) -> (u32, u32) {
    if auras::positive(s) {
        if area(s) {
            return (
                procs::PROC_DONE_POSITIVE_AOE,
                procs::PROC_TAKEN_POSITIVE_AOE,
            );
        }
        return (
            procs::PROC_DONE_POSITIVE_SPELL,
            procs::PROC_TAKEN_POSITIVE_SPELL,
        );
    }
    match s.dmg_class {
        2 => (procs::PROC_DONE_MELEE_SPELL, procs::PROC_TAKEN_MELEE_SPELL),
        3 => (
            procs::PROC_DONE_RANGED_SPELL,
            procs::PROC_TAKEN_RANGED_SPELL,
        ),
        _ if area(s) => (procs::PROC_DONE_AOE, procs::PROC_TAKEN_AOE),
        _ => (
            procs::PROC_DONE_NEGATIVE_SPELL,
            procs::PROC_TAKEN_NEGATIVE_SPELL,
        ),
    }
}

/// `SMSG_SPELLHEALLOG`, with its crit flag.
pub fn heal_log(
    caster: Guid,
    target: Guid,
    spell: u32,
    amount: u32,
    crit: bool,
) -> std::sync::Arc<[u8]> {
    let mut b = Body::default();
    crate::game::update::write_packed_guid(&mut b.0, target);
    crate::game::update::write_packed_guid(&mut b.0, caster);
    b.u32(spell).u32(amount).u8(u8::from(crit));
    b.frame(SMSG_SPELLHEALLOG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_hits_by_level() {
        assert_eq!(magic_hit_chance(60, 60, false, 0.0, 0.0, 0.0), 9_600);
        assert_eq!(magic_hit_chance(60, 62, false, 0.0, 0.0, 0.0), 9_400);
        // A raid boss three levels up: 83%.
        assert_eq!(magic_hit_chance(60, 63, false, 0.0, 0.0, 0.0), 8_300);
        assert_eq!(magic_hit_chance(60, 63, true, 0.0, 0.0, 0.0), 8_700);
        assert_eq!(magic_hit_chance(1, 60, false, 0.0, 0.0, 0.0), 2_200);
        assert_eq!(magic_hit_chance(60, 63, false, 3.0, 0.0, 0.0), 8_600);
        assert_eq!(magic_hit_chance(60, 60, false, 10.0, 0.0, 0.0), 9_900);
    }

    #[test]
    fn resistance_resists_in_buckets() {
        // 300 resistance at 60: 75%, the cap.
        assert!((resist_chance(300, 60, false, 0) - 0.75).abs() < 0.001);
        assert_eq!(resist_chance(0, 60, true, 0), 0.0);
        assert!(resist_chance(0, 60, true, 3) > 0.0);
        // At no chance nothing is resisted; at the cap, three quarters
        // most often.
        assert_eq!(resist_fraction(0.0, 5_000), 0.0);
        assert_eq!(resist_fraction(75.0, 1_000), 0.75);
    }
}
