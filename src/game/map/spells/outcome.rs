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
    /// Blocked whole, as a miss: an ability marked so (Shield Bash,
    /// `SPELL_ATTR_EX3_COMPLETELY_BLOCKED`).
    Blocked,
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

    pub(crate) fn wire(self) -> u8 {
        match self {
            Hit::Hit => 0,
            Hit::Miss => 1,
            Hit::Resist => 2,
            Hit::Dodge => 3,
            Hit::Parry => 4,
            Hit::Block | Hit::Blocked => 5,
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
            Hit::Blocked => procs::EX_BLOCK,
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
    /// How many share its first effect's damage (a meteor's), if any do.
    pub shared: u32,
    /// The spell was triggered.
    pub triggered: bool,
    /// An aura's or a proc's (`spells::cast_by_aura`).
    pub by_aura: bool,
    /// Cast from an item.
    pub item: bool,
}

impl Landing {
    /// Where `s`'s hit came from, for its procs.
    pub fn origin(&self, s: &SpellTemplate) -> procs::Origin {
        procs::Origin::of(s, self.by_aura, self.item)
    }
}

/// Whether spell outcomes are rolled (production), or fixed: always a hit,
/// never a crit or a partial resist below a certain chance (tests).
pub fn rolling(map: &Map) -> bool {
    map.spells.rolls.unwrap_or(map.cfg.spell_rolls)
}

/// A roll from 0 to 9,999, hundredths of a percent.
fn roll(map: &Map) -> i32 {
    #[cfg(test)]
    if let Some(r) = map
        .spells
        .queued_rolls
        .lock()
        .ok()
        .and_then(|mut q| q.pop_front())
    {
        return r;
    }
    if rolling(map) {
        rand::rng().random_range(0..10_000)
    } else {
        // Fixed: anything less than certain fails, and certainty holds.
        9_999
    }
}

/// Whether `e` puts on an aura of a kind the auras `a` keep off.
pub fn state_immune(a: &auras::Auras, e: &crate::game::data::spells::SpellEffect) -> bool {
    matches!(e.effect, 6 | 27 | 35) && a.state_immune(e.aura)
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

/// A spell a reflection may turn back: single-target magic whose
/// attributes don't forbid it, not an ability, a passive or a spell past
/// immunities (mangos's `IsReflectableSpell`, Thistle Tea's
/// `reflectable?`).
pub fn reflectable(s: &SpellTemplate) -> bool {
    !area(s)
        && s.dmg_class == 1
        && !s.attr(1, sd::ATTR_EX_NO_REFLECTION)
        && !s.attr(
            0,
            sd::ATTR_ABILITY | sd::ATTR_PASSIVE | sd::ATTR_NO_IMMUNITIES,
        )
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
    // A spell that pierces school immunities still meets an immunity to
    // damage (mangos's `CanPierceImmuneAura` against `IsImmuneToDamage`,
    // Thistle Tea's `DamageImmunity`).
    let mask = s.school_mask();
    if !s.attr(0, sd::ATTR_NO_IMMUNITIES)
        && ((!s.attr(2, sd::ATTR_EX2_NO_SCHOOL_IMMUNITIES)
            && a.immune_by(AURA_SCHOOL_IMMUNITY, mask))
            || a.immune_by(AURA_DAMAGE_IMMUNITY, mask))
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
        // An aura of a kind the target is immune to counts as immune too.
        let any = !mechanics.is_empty() || s.effects.iter().any(|e| state_immune(a, e));
        any && s.effects.iter().filter(|e| e.effect != 0).all(|e| {
            a.mechanic_immune(if e.mechanic != 0 {
                e.mechanic
            } else {
                s.mechanic
            }) || state_immune(a, e)
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
    if reflectable(s) {
        let reflect = a.sum(AURA_REFLECT_SPELLS)
            + a.sum_mask(AURA_REFLECT_SPELLS_SCHOOL, s.school_mask())
            + ward_reflect(map, target, s);
        if reflect > 0 && chance(map, reflect as f32) {
            return Hit::Reflect;
        }
    }
    // A spell of no damage class never misses (mangos-classic's
    // `SpellHitResult`, Thistle Tea's unclassified spells): no magic roll.
    if s.dmg_class == 0 {
        return Hit::Hit;
    }
    // A creature with no defence against spells misses none, whatever its
    // level and mechanic resistance (`NO_SPELL_DEFENSE`, as Thistle Tea's
    // `SpellResist.magic_hit?`).
    if crate::game::map::creature_combat::no_spell_defense(map, target) {
        return Hit::Hit;
    }
    let chance = spell_hit_chance(map, caster, target, s, m);
    if roll_strict(map, chance) {
        Hit::Hit
    } else {
        Hit::Resist
    }
}

/// The chance, in hundredths of a percent, that magic spell `s` from
/// `caster` lands on `target` (before immunities and reflection).
pub fn spell_hit_chance(map: &Map, caster: Guid, target: Guid, s: &SpellTemplate, m: &Mods) -> i32 {
    let Some(a) = auras::auras(map, target) else {
        return 10_000;
    };
    let caster_level = level(map, caster);
    let target_level = level(map, target);
    let d = derived(map, caster);
    let hit_bonus = d.spell_hit + m.flat[mods::OP_RESIST_MISS_CHANCE] as f32;
    // The spell's own mechanic in its hit; an effect's of another rolls
    // apart (`effect_resisted`).
    let mechanic = if s.mechanic != 0 {
        a.sum_misc(AURA_MOD_MECHANIC_RESISTANCE, s.mechanic as i32) as f32
    } else {
        0.0
    };
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
    // The target's own change to the chance of its attackers' spells of the
    // school (mangos's `MagicSpellHitChance`, Thistle Tea's
    // `attacker_spell_hit_chance`).
    let versus = a.sum_mask(AURA_MOD_ATTACKER_SPELL_HIT_CHANCE, s.school_mask()) as f32;
    magic_hit_chance(
        caster_level,
        target_level,
        guid::is_player(target),
        hit_bonus + versus - aoe,
        mechanic,
        binary_resist,
    )
}

/// Whether `target` resists effect `index` of harmful `s` apart from the
/// spell, as Thistle Tea's `MechanicResistance.effect_resisted?` has it: an
/// effect whose mechanic isn't the spell's rolls its own resistance.
pub fn effect_resisted(map: &Map, target: Guid, s: &SpellTemplate, index: usize) -> bool {
    let mechanic = s.effects[index].mechanic;
    if mechanic == 0
        || mechanic == s.mechanic
        || crate::game::map::creature_combat::no_spell_defense(map, target)
    {
        return false;
    }
    let chance = auras::auras(map, target).map_or(0, |a| {
        a.sum_misc(AURA_MOD_MECHANIC_RESISTANCE, mechanic as i32)
    });
    chance > 0 && roll(map) < chance * 100
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
/// Whether a chance of `chance_bp` hundredths of a percent comes up.
pub fn hits(map: &Map, chance_bp: i32) -> bool {
    roll_strict(map, chance_bp)
}

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
    // An ability of no weapon (a ranged one not of the ranged slot: Scorpid
    // Sting, Wing Clip's kind) rolls with the most skill of its caster's
    // level, not a weapon's (mangos-classic's `IsSpellUseWeaponSkill` and
    // `GetSkillMaxForLevel`; Thistle Tea's level skill for one needing no
    // weapon).
    if !uses_weapon_skill(s) {
        r.attacker_skill = r.attacker_level * 5;
    }
    r.crit += m.flat[mods::OP_CRITICAL_CHANCE] as f32;
    r.can_crit = !s.attr(2, sd::ATTR_EX2_CANT_CRIT);
    if s.attr(0, 0x20_0000) {
        r.avoidance_disabled = true;
    }
    // Its mechanic resisted, in the band just past the miss chance
    // (Thistle Tea's `roll_special_outcome`).
    let first = roll(map);
    let resist = auras::auras(map, target).map_or(0, |a| {
        std::iter::once(s.mechanic)
            .chain(s.effects.iter().map(|e| e.mechanic))
            .filter(|m| *m != 0)
            .map(|mech| a.sum_misc(AURA_MOD_MECHANIC_RESISTANCE, mech as i32))
            .max()
            .unwrap_or(0)
            * 100
    });
    let miss = r.miss_bp();
    if resist > 0 && first >= miss && first < miss + resist {
        return (Hit::Resist, false);
    }
    match combat::roll_special(&r, first, roll(map)) {
        combat::Outcome::Miss => (Hit::Miss, false),
        combat::Outcome::Dodge => (Hit::Dodge, false),
        combat::Outcome::Parry => (Hit::Parry, false),
        combat::Outcome::Block => (block_of(s), false),
        combat::Outcome::Crit => (Hit::Hit, true),
        _ => (Hit::Hit, false),
    }
}

/// Whether ability `s` rolls with its weapon's skill: one wanting a weapon,
/// a melee one, or a ranged one of the ranged slot (mangos-classic's
/// `IsSpellUseWeaponSkill`).
pub fn uses_weapon_skill(s: &SpellTemplate) -> bool {
    s.equipped_item_class == 2
        || s.dmg_class == 2
        || (s.dmg_class == 3 && s.attr(0, sd::ATTR_RANGED))
}

/// What a block of ability `s` is: a miss for one blocked whole (Shield
/// Bash, `SPELL_ATTR_EX3_COMPLETELY_BLOCKED`), its effects all lost; else
/// a hit blocked in part (mangos-classic's `CanBlockAbility`).
pub fn block_of(s: &SpellTemplate) -> Hit {
    if s.attr(3, sd::ATTR_EX3_COMPLETELY_BLOCKED) {
        Hit::Blocked
    } else {
        Hit::Block
    }
}

/// Whether a damage or heal effect of `s` crits.
pub fn roll_crit(map: &Map, caster: Guid, target: Guid, s: &SpellTemplate, m: &Mods) -> bool {
    if s.attr(2, sd::ATTR_EX2_CANT_CRIT) {
        return false;
    }
    let d = derived(map, caster);
    let mut c = if super::class_scripts::melee_crit(s) {
        d.crit
    } else if physical(s) {
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
    chance(map, c)
}

/// What a crit's bonus gains against the target's creature type, in
/// percent (Monster Slaying, Humanoid Slaying: aura 169 is the crit's
/// damage, not its chance, as vmangos's `SpellCriticalDamageBonus`, mangos's
/// `GetCritMultiplier` and Thistle Tea's `TargetAttackPower` have it).
pub fn crit_versus(map: &Map, caster: Guid, target: Guid) -> f32 {
    // Its type's bit: a player's a humanoid's, or a beast's in a beast's
    // form (`combat::creature_type_mask`).
    let bit = combat::creature_type_mask(map, target);
    if bit == 0 {
        return 0.0;
    }
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
pub fn shatter(map: &Map, caster: Guid, s: &SpellTemplate) -> f32 {
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
        let ticks = aura_max_ticks(s, duration);
        let factor = if ticks > 0 { base / ticks as f32 } else { base };
        c = time / 3500.0 * factor;
    }
    c * level_penalty(s)
}

/// The ticks a coefficient over time is shared by (vmangos's
/// `GetAuraMaxTicks`, Thistle Tea's `max_ticks`): the first aura of damage,
/// healing or a leech over time's, over at most 30 seconds; one for no
/// duration; six for an aura of none of those (a funnel of health's).
pub fn aura_max_ticks(s: &SpellTemplate, duration: u32) -> u32 {
    if duration == 0 {
        return 1;
    }
    let duration = duration.min(30_000);
    s.effects
        .iter()
        .find(|x| {
            x.effect == 6
                && matches!(
                    x.aura,
                    AURA_PERIODIC_DAMAGE | AURA_PERIODIC_HEAL | AURA_PERIODIC_LEECH
                )
        })
        .and_then(|x| duration.checked_div(x.amplitude))
        .unwrap_or(6)
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
        // Those of the heal's school (mangos's `SpellBaseHealingBonusDone`
        // by its school mask).
        let done = a.map_or(0, |a| a.sum_mask(AURA_MOD_HEALING_DONE, s.school_mask()));
        let from_spirit = a.map_or(0, |a| {
            spirit * a.sum(AURA_MOD_SPELL_HEALING_OF_STAT_PERCENT) / 100
        });
        return (done + from_spirit.max(0)) as f32;
    }
    let mut p = d.spell_power[school_index(s)] as f32;
    let bit = combat::creature_type_mask(map, target);
    if bit != 0
        && let Some(a) = auras::auras(map, caster)
    {
        // Gear's and potions' spell damage against a creature type (aura
        // 180, "up to", as spell power: Thistle Tea's `TargetSpellPower`).
        // Damage done against one (aura 59) is flat ([`bonus`]).
        p += a
            .effects()
            .filter(|(_, x)| {
                x.kind == AURA_MOD_FLAT_SPELL_DAMAGE_VERSUS && (x.misc as u32) & bit != 0
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
    if s.attr(3, sd::ATTR_EX3_IGNORE_CASTER_MODIFIERS) {
        return 0.0;
    }
    let heal = matches!(s.effects[index].effect, 10) || s.effects[index].aura == AURA_PERIODIC_HEAL;
    // Damage done against the target's creature type (aura 59), flat: a
    // spell's whole; a weapon's blow's whole after the weapon's
    // percentages, by the hand's damage done; another ability's (a school
    // damage effect, one over time) by its coefficient (mangos's
    // `SpellDamageBonusDone` and `MeleeDamageBonusDone`, through
    // `SpellBonusWithCoeffs`; Thistle Tea's `TargetDamage`).
    let versus = if heal {
        0.0
    } else {
        combat::damage_versus(map, caster, target)
    };
    if physical(s) {
        if versus == 0.0 {
            return 0.0;
        }
        let weapon_based = kind == Kind::Direct && !s.has_effect(2);
        return if weapon_based {
            let hand = if s.dmg_class == 3 {
                combat::Hand::Ranged
            } else if s.attr(3, sd::ATTR_EX3_REQ_OFFHAND) {
                combat::Hand::Off
            } else {
                combat::Hand::Main
            };
            versus * combat::hand_damage_pct(map, caster, hand)
        } else {
            let table = s.effects[index.min(2)].bonus_coefficient;
            let creature = map.creature(caster).is_some()
                && crate::game::map::pets::master(map, caster) == caster;
            versus
                * if table > 0.0 {
                    table
                } else if creature {
                    1.0
                } else {
                    0.0
                }
        };
    }
    let p = power(map, caster, target, s, heal);
    if p <= 0.0 {
        return versus;
    }
    let c = coefficient(map, s, index, kind) * (100 + m.pct[mods::OP_SPELL_BONUS_DAMAGE]) as f32
        / 100.0;
    p * c + m.flat[mods::OP_SPELL_BONUS_DAMAGE] as f32 + versus
}

/// The caster's multiplier on damage of `s`'s school done to `target`:
/// none for a spell that ignores its caster's modifiers (Thaddius's
/// charges).
pub fn done_multiplier(map: &Map, caster: Guid, target: Guid, s: &SpellTemplate) -> f32 {
    if s.attr(3, sd::ATTR_EX3_IGNORE_CASTER_MODIFIERS) {
        return 1.0;
    }
    let Some(a) = auras::auras(map, caster) else {
        return 1.0;
    };
    // Physical damage's percent is in the weapon's damage already.
    let mut m = if physical(s) && s.school == 0 {
        1.0
    } else {
        crate::game::map::derive::spell_pct(a, s.school_mask())
    };
    let bit = combat::creature_type_mask(map, target);
    if bit != 0 {
        m *= a.product_mask(AURA_MOD_DAMAGE_DONE_VERSUS, bit);
    }
    m
}

/// Damage `target` takes of `s`: its flat and percent modifiers, the flat
/// whole.
pub fn taken(map: &Map, target: Guid, s: &SpellTemplate, damage: f32) -> f32 {
    taken_with(map, target, s, damage, None)
}

/// The share of a target's flat damage taken (Dampen and Amplify Magic) a
/// magic effect gets, as mangos's `SpellBonusWithCoeffs` on the taken part
/// and Thistle Tea's `DamageReceived` have it: its bonus coefficient for a
/// player's (or a player's pet's) spell, a creature's whole unless its
/// table says otherwise; a melee or ranged ability's whole.
pub fn taken_coefficient(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    kind: Kind,
) -> f32 {
    if physical(s) || s.custom(sd::CUSTOM_FIXED_DAMAGE) {
        return 1.0;
    }
    let creature =
        map.creature(caster).is_some() && crate::game::map::pets::master(map, caster) == caster;
    let table = s.effects[index.min(2)].bonus_coefficient;
    if creature {
        if table > 0.0 { table } else { 1.0 }
    } else {
        coefficient(map, s, index.min(2), kind)
    }
}

/// Damage `target` takes of `caster`'s effect `index` of `s`, a direct
/// hit's or a tick's (`kind`) of `stacks`: a spell's flat modifiers (Dampen
/// Magic, Judgement of the Crusader) by [`taken_coefficient`] and taking at
/// most half, as Thistle Tea's `DamageReceived` and mangos's
/// `SpellDamageBonusTaken` have them; an ability's whole; then the percent
/// modifiers.
#[allow(clippy::too_many_arguments)]
pub fn taken_of(
    map: &Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    kind: Kind,
    stacks: u32,
    damage: f32,
) -> f32 {
    let scale = (!physical(s) && !s.custom(sd::CUSTOM_FIXED_DAMAGE))
        .then(|| taken_coefficient(map, caster, s, index, kind) * stacks.max(1) as f32);
    taken_with(map, target, s, damage, scale)
}

/// Nothing for a spell whose attributes ignore the target's modifiers
/// (Decimate's, mangos's `IGNORE_DAMAGE_TAKEN_MODIFIERS`).
fn taken_with(map: &Map, target: Guid, s: &SpellTemplate, damage: f32, scale: Option<f32>) -> f32 {
    if s.attr(4, sd::ATTR_EX4_IGNORE_DAMAGE_TAKEN_MODIFIERS) {
        return damage.max(0.0);
    }
    let Some(a) = auras::auras(map, target) else {
        return damage;
    };
    let mask = s.school_mask();
    let mut flat = a.sum_mask(AURA_MOD_DAMAGE_TAKEN, mask) as f32;
    let mut pct = a.product_mask(AURA_MOD_DAMAGE_PERCENT_TAKEN, mask);
    // A melee or ranged ability takes the melee or ranged damage taken
    // auras too, whatever their school (theirs is 0: Stoneskin, Windwall),
    // as a swing does: every flat before every percentage, and nothing
    // added to no damage at all (mangos's `MeleeDamageBonusTaken`, Thistle
    // Tea's `AttackDamageTaken.spell_amount`).
    match s.dmg_class {
        2 => {
            flat += a.sum(AURA_MOD_MELEE_DAMAGE_TAKEN) as f32;
            pct *= a.product(AURA_MOD_MELEE_DAMAGE_TAKEN_PCT);
        }
        3 => {
            flat += a.sum(AURA_MOD_RANGED_DAMAGE_TAKEN) as f32;
            pct *= a.product(AURA_MOD_RANGED_DAMAGE_TAKEN_PCT);
        }
        _ => {}
    }
    let d = match scale {
        Some(c) => damage + (flat * c).max(-damage.max(0.0) / 2.0),
        None if damage <= 0.0 => return 0.0,
        None => damage + flat,
    };
    (d * pct).max(0.0)
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
    // A fixed amount takes no bonus or modifier, done or taken (vmangos's
    // custom flag).
    let fixed = s.custom(sd::CUSTOM_FIXED_DAMAGE);
    let mut d = if fixed {
        base
    } else {
        base + bonus(map, caster, target, s, index, Kind::Direct, &l.mods)
    };
    d *= if l.chain > 0.0 { l.chain } else { 1.0 };
    if !fixed {
        d *= done_multiplier(map, caster, target, s);
        d = l.mods.apply(mods::OP_DAMAGE, d);
    }
    // The target's modifiers, the crit, then armor (mangos's
    // `CalculateSpellDamage`; Thistle Tea's "physical spells apply target
    // bonuses and critical damage before armor").
    if !fixed {
        d = taken_of(map, caster, target, s, index, Kind::Direct, 1, d);
    }
    if l.crit {
        let pct = l.mods.pct[mods::OP_CRIT_DAMAGE_BONUS] as f32 / 100.0;
        // Against the target's type, the bonus is more (a white blow's too,
        // `combat::crit_multiplier`).
        let versus = crit_versus(map, caster, target);
        let bonus = if physical(s) {
            let melee =
                auras::auras(map, caster).map_or(0, |a| a.sum(AURA_MOD_CRIT_DAMAGE_BONUS_MELEE));
            d * (1.0 + pct) * (100.0 + melee as f32 + versus) / 100.0
        } else {
            d * 0.5 * (1.0 + pct) * (100.0 + versus) / 100.0
        };
        d += bonus;
    }
    if s.school == 0 && !s.custom(sd::CUSTOM_IGNORE_ARMOR) {
        // What armor takes off, whole, as a white blow's (Thistle Tea's
        // `armor_reduced_damage`): 1000 at 14.003% leaves 860, not 859.
        let armor = resistance(map, caster, target, 0);
        d -= (d * combat::armor_reduction(armor as u32, level(map, caster))).trunc();
    }
    let mut blocked = 0;
    if l.hit == Hit::Block {
        blocked = crate::game::map::combat::block_value(map, target).min(d as u32);
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
    if s.school == 0 || binary(s) {
        return 0;
    }
    resisted_of_school(map, caster, target, school_index(s), damage, tick)
}

/// Damage of the magic school `school` (an index) from `caster` resisted in
/// part by `target`'s resistance, less the caster's penetration: a spell's
/// or an elemental's blow (mangos's `CalculateDamageAbsorbAndResist`), a
/// tenth as often for a tick.
pub fn resisted_of_school(
    map: &Map,
    caster: Guid,
    target: Guid,
    school: usize,
    damage: u32,
    tick: bool,
) -> u32 {
    if damage == 0 || school == 0 || !rolling(map) {
        return 0;
    }
    let caster_level = level(map, caster);
    let diff = level(map, target) as i32 - caster_level as i32;
    let r = resistance(map, caster, target, school);
    let mut c = resist_chance(r, caster_level, !guid::is_player(target), diff) * 100.0;
    if tick {
        c *= 0.1;
    }
    (damage as f32 * resist_fraction(c, roll(map))) as u32
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
    // A fixed amount takes no bonus or modifier, done or taken, as fixed
    // damage doesn't (vmangos's custom flag, Thistle Tea's "fixed damage
    // and healing keep base modifiers but bypass outgoing bonuses").
    if s.custom(sd::CUSTOM_FIXED_DAMAGE) {
        let h = base * if l.chain > 0.0 { l.chain } else { 1.0 };
        return (if l.crit { h * 1.5 } else { h }).max(0.0) as u32;
    }
    let mut h = base + bonus(map, caster, target, s, index, Kind::Direct, &l.mods);
    h *= if l.chain > 0.0 { l.chain } else { 1.0 };
    if let Some(a) = auras::auras(map, caster)
        && !s.attr(3, sd::ATTR_EX3_IGNORE_CASTER_MODIFIERS)
    {
        h *= a.product(AURA_MOD_HEALING_DONE_PERCENT);
    }
    h = l.mods.apply(mods::OP_DAMAGE, h);
    let chain = if l.chain > 0.0 { l.chain } else { 1.0 };
    h = spell_healing_taken(map, target, h, s, index, Kind::Direct, 1, chain);
    if l.crit {
        h *= 1.5;
    }
    h.max(0.0) as u32
}

/// Healing of `amount` from effect `index` of `s` that `target` takes, as
/// it lands (a heal over time's at each tick; Thistle Tea's
/// `HealingReceived.spell_amount`). A spell of a damage class takes the
/// target's flat healing taken for its school by its coefficient for
/// `kind`, its `stacks` and its `chain` falloff, losing at most half, and
/// Healing Way's gain on a Healing Wave; then, as for any heal, the
/// strongest percentage cut and gain.
#[allow(clippy::too_many_arguments)]
pub fn spell_healing_taken(
    map: &Map,
    target: Guid,
    amount: f32,
    s: &SpellTemplate,
    index: usize,
    kind: Kind,
    stacks: u32,
    chain: f32,
) -> f32 {
    let amount = amount.max(0.0);
    let Some(a) = auras::auras(map, target).filter(|_| matches!(s.dmg_class, 1..=3)) else {
        return healing_taken(map, target, amount);
    };
    let mut benefit = a.sum_mask(AURA_MOD_HEALING, s.school_mask()) as f32;
    // Blessing of Light: more taken from a Holy Light or a Flash of Light
    // (1.12's direct heals of the paladins' family), by its coefficient
    // (mangos's `SpellHealingBonusTaken`).
    if kind == Kind::Direct && s.family_name == 10 && s.family_flags & 0xC000_0000 != 0 {
        let holy_light = s.family_flags & 0x8000_0000 != 0;
        benefit += super::scripts::blessing_of_light_points(map, target, holy_light) as f32;
    }
    let flat = (benefit * coefficient(map, s, index, kind) * stacks.max(1) as f32 * chain)
        .max(-amount / 2.0);
    // Healing Way: more from each Healing Wave on the target.
    let mut way = 1.0;
    if s.family_name == 11 && s.family_flags & 0x40 != 0 {
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
        way = (100 + pct).max(0) as f32 / 100.0;
    }
    healing_taken(map, target, (amount + flat) * way)
}

/// Healing of `amount` that `target` takes (Thistle Tea's
/// `HealingReceived.amount`): only its strongest percentage cut and its
/// strongest gain.
pub fn healing_taken(map: &Map, target: Guid, amount: f32) -> f32 {
    match auras::auras(map, target) {
        Some(a) => {
            let cut = a.min(AURA_MOD_HEALING_PCT).min(0);
            let gain = a.max(AURA_MOD_HEALING_PCT).max(0);
            (amount * (100 + cut).max(0) as f32 / 100.0 * (100 + gain) as f32 / 100.0).max(0.0)
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
    // Its damage puts a creature in the fight, whether it saw the caster
    // or not, another spell's trigger's too (a trap's); an aura's or a
    // proc's doesn't, unless it is no proc's own (Thistle Tea's
    // `damage_contact?`; mangos's `DealDamage` for every one). The players'
    // sides of it too (mangos's `HandleDamageDealt`).
    // An object's (a brazier's) is no one's fight (mangos's affective
    // caster: none for an object no one owns).
    if d.amount > 0
        && caster != target
        && (guid::is_unit(caster) || guid::is_player(caster))
        && !(l.by_aura && !s.attr(3, sd::ATTR_EX3_NOT_A_PROC))
    {
        if guid::is_unit(target) {
            crate::game::map::ai::attacked(map, target, caster);
        }
        combat::struck(map, caster, target);
    }
    let (left, absorbed) = auras::absorb_and_share(map, caster, target, d.amount, s.school_mask());
    let frame = damage_log(
        caster, target, s.id, s.school, left, absorbed, d.resisted, d.blocked, l.crit,
    );
    map.send_to_observers(caster, &frame, true);
    if let Some(p) = map.players.get(&target)
        && !p.known.contains(&caster)
    {
        p.outbound.send_frame(frame);
    }
    // Absorbed whole, it stands a seated target still.
    combat::stand_struck(map, target);
    if left > 0 {
        // What an absorb took pushes nothing back (Thistle Tea's
        // `take_unblocked_damage`, mangos's `DealDamage`).
        super::pushback_by(map, target, caster);
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
    // With the spell's other effects on the target, once (`effects::run`).
    procs::spell_hit(
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
            origin: l.origin(s),
        },
    );
    let items = death_items(map, target, caster, left);
    if left > 0 {
        // Its threat by the auras of its school (`super::threat`).
        combat::deal_damage_by(map, caster, target, left, s.school_mask(), Some(s.id));
    }
    give_death_items(map, target, items);
    super::threat(map, caster, target, s, left as f32, &l.mods, l.crit);
}

/// The items a unit's death under `AURA_CHANNEL_DEATH_ITEM` leaves its
/// casters (Drain Soul's and Shadowburn's soul shards), if `damage` from
/// `by` kills it: only for a death that gives experience or honor, the
/// killing blow's part in it counted.
pub fn death_items(map: &Map, target: Guid, by: Guid, damage: u32) -> Vec<(Guid, u32, u32)> {
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
    let data = &map.realm.data;
    let target_level = level(map, target);
    let creature = map.creature(target);
    // A soul shard only from a death worth something to its caster (Thistle
    // Tea's `DeathItem.eligible?`, mangos's `isHonorOrXPTarget`): not grey,
    // a player or tapped, not a pet or a totem, nor a creature worth no
    // experience, nor one mostly the NPCs' kill, its killing blow counted
    // (Thistle Tea's `DamageOrigin` before the shard).
    let shard_target = creature.is_none_or(|c| {
        c.pet.is_none()
            && crate::game::map::creature_combat::loot_allowed_with(map, target, by, damage)
            && crate::game::map::pets::owner_of(map, target).is_none()
            && data
                .creature_template(c.entry)
                .is_none_or(|t| t.xp_rate() > 0.0 && t.static_flags1 & 0x2 == 0)
    });
    let mut items: Vec<(Guid, u32, u32)> = Vec::new();
    for (h, x) in a.effects() {
        let item = u32::try_from(x.mask).unwrap_or(0);
        // Nothing for no item or none of it (mangos's
        // `HandleChannelDeathItem`).
        if x.kind != AURA_CHANNEL_DEATH_ITEM || item == 0 || x.amount <= 0 {
            continue;
        }
        // A creature tapped by the caster or their group, the killing
        // blow taking an untapped one (mangos's `HandleChannelDeathItem`,
        // `IsTappedBy`).
        if item == SOUL_SHARD {
            let tapped = guid::is_player(target)
                || creature
                    .is_some_and(|c| crate::game::map::tap::tapped_at_death(map, c, by, h.caster));
            if !shard_target || !tapped || target_level <= combat::gray_level(level(map, h.caster))
            {
                continue;
            }
        }
        // One shard a warlock, however many of their drains were on it.
        let warlock = data.spells.get(h.spell).is_some_and(|s| s.family_name == 5);
        if warlock && items.iter().any(|(c, i, _)| *c == h.caster && *i == item) {
            continue;
        }
        items.push((h.caster, item, x.amount as u32));
    }
    items
}

/// Soul Shard.
const SOUL_SHARD: u32 = 6265;

pub fn give_death_items(map: &mut Map, target: Guid, items: Vec<(Guid, u32, u32)>) {
    if items.is_empty() || map.unit_view(target).is_some_and(|u| u.alive) {
        return;
    }
    use crate::game::map::inventory;
    let data = map.realm.data.clone();
    for (caster, item, count) in items {
        // What fits is given, what doesn't told of (mangos's
        // `HandleChannelDeathItem`: `count - noSpaceForCount`, and the
        // equip error); none fitting, nothing.
        let Some(p) = map.players.get(&caster) else {
            continue;
        };
        let fits = match inventory::fitting(p, &data, item, count) {
            Ok(n) => n,
            Err(why) => {
                inventory::send_fail(p, why, Guid::zero());
                continue;
            }
        };
        if fits < count
            && let Err(why) = inventory::room_for(p, &data, item, count)
        {
            inventory::send_fail(p, why, Guid::zero());
        }
        let _ = inventory::add_item(map, caster, item, fits, true);
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
