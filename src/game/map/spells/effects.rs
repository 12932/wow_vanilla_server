//! A spell's effects on a target once it has landed: damage and healing
//! through [`super::outcome`], a spell's auras as one holder, weapon damage
//! as one blow, and the rest; effects at a place (persistent areas,
//! missiles); and the party auras that follow their casters.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{Power, SMSG_SPELLENERGIZELOG, Vector3d};

use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, dist};

use super::mods::{self, Mods};
use super::outcome::{self, Kind, Landing};
use super::*;

/// Diminishing returns' groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Dr {
    ControlledStun = 1,
    TriggeredStun,
    KidneyShot,
    Sleep,
    Polymorph,
    ControlledRoot,
    Fear,
    WarlockFear,
    Charm,
    Silence,
    Disarm,
    Freeze,
    Knockout,
    Banish,
    Horror,
}

/// A spell's diminishing returns group, as Thistle Tea follows vmangos.
fn dr_group(s: &SpellTemplate) -> Option<Dr> {
    let flags = s.family_flags as u32;
    match (s.family_name, s.id) {
        (8, _) if flags & 0x20_0000 != 0 => return Some(Dr::KidneyShot),
        (8, _) if flags & 0x100_0000 != 0 => return None,
        (9, _) if flags & 0x8 != 0 => return Some(Dr::Freeze),
        (5, 6358) => return Some(Dr::WarlockFear),
        (5, _) if flags & 0x8000_0000 != 0 && s.mechanic == 5 => return Some(Dr::WarlockFear),
        (5, _) if flags & 0x8000_0000 != 0 => return None,
        (4, _) if flags & 0x2 != 0 => return None,
        (11, _) if flags & 0x8000_0000 != 0 => return Some(Dr::ControlledRoot),
        (3, _) if s.spell_visual == 4325 => return None,
        (0, 12355 | 18093) => return Some(Dr::TriggeredStun),
        (_, 7922 | 20253 | 20614 | 20615) => return Some(Dr::ControlledStun),
        _ => {}
    }
    let mechanics: Vec<u32> = std::iter::once(s.mechanic)
        .chain(s.effects.iter().map(|e| e.mechanic))
        .collect();
    [
        (12, Dr::ControlledStun),
        (10, Dr::Sleep),
        (17, Dr::Polymorph),
        (7, Dr::ControlledRoot),
        (5, Dr::Fear),
        (1, Dr::Charm),
        (9, Dr::Silence),
        (3, Dr::Disarm),
        (13, Dr::Freeze),
        (14, Dr::Knockout),
        (30, Dr::Knockout),
        (18, Dr::Banish),
        (24, Dr::Horror),
    ]
    .into_iter()
    .find(|(m, _)| mechanics.contains(m))
    .map(|(_, g)| g)
}

/// The duration a crowd control lasts on `target` after its diminishing
/// returns: whole, a half, a quarter, then immune (`None`) for 15 seconds.
fn diminished(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    duration: Option<u32>,
) -> Option<Option<u32>> {
    let Some(ms) = duration else {
        return Some(duration);
    };
    let Some(group) = dr_group(s) else {
        return Some(duration);
    };
    let all = matches!(
        group,
        Dr::ControlledStun | Dr::TriggeredStun | Dr::KidneyShot
    );
    // The rest only between players (and what players control), as
    // Thistle Tea's `DiminishingReturns.applicable?` has it.
    let player_side = |g: Guid| {
        guid::is_player(g) || crate::game::map::pets::owner_of(map, g).is_some_and(guid::is_player)
    };
    if !all && !(player_side(target) && player_side(caster)) {
        return Some(duration);
    }
    let now = map.now;
    let on = group_on(map, target, group);
    let c = map.caster_mut(target)?;
    // Recovered once its time since the group's last aura went is up; no
    // clock while one is on.
    let entry = match c
        .diminishing
        .iter_mut()
        .position(|(g, _, _)| *g == group as u8)
    {
        Some(i) => {
            if !on && c.diminishing[i].2.is_some_and(|r| r <= now) {
                c.diminishing[i] = (group as u8, 0, None);
            }
            &mut c.diminishing[i]
        }
        None => {
            c.diminishing.push((group as u8, 0, None));
            c.diminishing.last_mut().expect("just pushed")
        }
    };
    if entry.1 >= 3 {
        return None;
    }
    let applications = entry.1;
    entry.1 += 1;
    entry.2 = None;
    Some(Some(ms >> applications))
}

/// How long a diminishing-returns group takes to recover once its last
/// aura is gone.
const DR_RECOVERY: Duration = Duration::from_secs(15);

/// Whether a harmful aura of diminishing `group` is on `target`.
fn group_on(map: &Map, target: Guid, group: Dr) -> bool {
    let data = &map.realm.data;
    auras::auras(map, target).is_some_and(|a| {
        a.holders
            .iter()
            .any(|h| !h.positive && data.spells.get(h.spell).and_then(dr_group) == Some(group))
    })
}

/// An aura came off `target`: if it was the last of its diminishing
/// group there, the group starts to recover.
pub fn dr_aura_gone(map: &mut Map, target: Guid, spell: u32) {
    let Some(group) = map.realm.data.spells.get(spell).and_then(dr_group) else {
        return;
    };
    if group_on(map, target, group) {
        return;
    }
    let now = map.now;
    if let Some(c) = map.caster_mut(target)
        && let Some(e) = c.diminishing.iter_mut().find(|(g, _, _)| *g == group as u8)
    {
        e.2 = Some(now + DR_RECOVERY);
    }
}

/// A finisher's duration at `combo` points: from the base at none to the
/// most at five, a fifth of the way for each, as mangos's
/// `CalculateSpellDuration` and Thistle Tea's `duration_for_combo_points`;
/// no further past five.
pub fn combo_duration(base: u32, max: u32, combo: u8) -> u32 {
    base + max.saturating_sub(base) * u32::from(combo.min(5)) / 5
}

/// An aura's duration: the spell's at the caster's level, longer for a
/// finishing move's combo points, with the caster's modifiers.
fn aura_duration(map: &Map, caster: Guid, s: &SpellTemplate, m: &Mods, combo: u8) -> Option<u32> {
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    let base = map.realm.data.spells.duration(s, level)?;
    let mut ms = base;
    if combo > 0
        && let Some(d) = map.realm.data.spells.durations.get(&s.duration_index)
        && d.max > base as i32
    {
        ms = combo_duration(base, d.max as u32, combo);
    }
    Some(m.apply_ms(mods::OP_DURATION, ms))
}

/// What an aura effect holds: for a periodic damage or heal, a tick's worth
/// with spell power and the caster's modifiers. What the target takes of a
/// heal's tick is worked out at each tick (`auras::tick`).
fn aura_amount(
    map: &Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    base: i32,
    l: &Landing,
) -> i32 {
    let e = &s.effects[index];
    if let Some(amount) = crate::game::map::item_scripts::aura_amount(s) {
        return amount;
    }
    match e.aura {
        AURA_PERIODIC_DAMAGE | AURA_PERIODIC_LEECH => {
            let mut d =
                base as f32 + outcome::bonus(map, caster, target, s, index, Kind::Dot, &l.mods);
            d *= outcome::done_multiplier(map, caster, target, s);
            l.mods.apply(mods::OP_DOT, d).max(0.0) as i32
        }
        AURA_PERIODIC_HEAL => {
            let mut h =
                base as f32 + outcome::bonus(map, caster, target, s, index, Kind::Dot, &l.mods);
            if let Some(a) = auras::auras(map, caster)
                && !s.attr(3, sd::ATTR_EX3_IGNORE_CASTER_MODIFIERS)
            {
                h *= a.product(AURA_MOD_HEALING_DONE_PERCENT);
            }
            l.mods.apply(mods::OP_DOT, h).max(0.0) as i32
        }
        AURA_SCHOOL_ABSORB => base + absorb_bonus(map, caster, target, s, index),
        _ => base,
    }
}

/// Thistle Tea's `AbsorbBonus`: Power Word: Shield gets a tenth of healing
/// power, the mage's elemental wards and Shadow Ward a tenth of their
/// school's spell power, less below level 20.
fn absorb_bonus(map: &Map, caster: Guid, target: Guid, s: &SpellTemplate, index: usize) -> i32 {
    let flags = s.family_flags as u32;
    let power = if s.family_name == 6 && flags & 0x1 != 0 {
        outcome::power(map, caster, target, s, true)
    } else if (s.family_name == 3 && flags & 0x108 != 0)
        || (matches!(s.family_name, 0 | 5)
            && s.school == 5
            && s.spell_icon == 207
            && s.category == 56)
    {
        outcome::power(map, caster, target, s, false)
    } else {
        return 0;
    };
    let _ = index;
    (power.max(0.0) * 0.1 * outcome::level_penalty(s)) as i32
}

/// The effects of `s` that land on `target` (those `which` marks).
#[allow(clippy::too_many_arguments)]
pub fn run(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    which: [bool; 3],
    l: &Landing,
    chain: &mut [f32; 3],
    item: Option<Guid>,
    t: &Targets,
) {
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    let mut aura_effects = Vec::new();
    let mut weapon = Vec::new();
    let mut pet_aura = Vec::new();
    for index in 0..3 {
        if !which[index] {
            continue;
        }
        // A totem shrugs off most of what others cast on it.
        if crate::game::map::pets::immune(map, caster, target, s, index)
            || crate::game::map::extra_effects::effect_immune(map, target, s, index)
        {
            continue;
        }
        let e = s.effects[index];
        let mut points = l
            .mods
            .points(s, index, points_with(s, index, level, l.combo));
        if index == 0 && l.shared > 1 {
            points /= l.shared as i32;
        }
        let mut here = l.clone();
        here.chain = chain[index];
        match e.effect {
            // Premeditation's hold on its points goes on with them (see
            // `retain_combo_points`).
            EFFECT_APPLY_AURA if e.aura == AURA_RETAIN_COMBO_POINTS && awards_combo_points(s) => {}
            EFFECT_APPLY_AURA | EFFECT_APPLY_AREA_AURA_PARTY if e.aura != 0 => {
                let amount = aura_amount(map, caster, target, s, index, points, &here);
                aura_effects.push((index, amount));
            }
            EFFECT_WEAPON_DAMAGE
            | EFFECT_WEAPON_DAMAGE_NOSCHOOL
            | EFFECT_NORMALIZED_WEAPON_DMG
            | EFFECT_WEAPON_PERCENT_DAMAGE => weapon.push((index, points)),
            // On the pet together, after (`pets::area_aura`).
            crate::game::map::pets::EFFECT_APPLY_AREA_AURA_PET
                if !crate::game::map::battleground::plants(s) =>
            {
                pet_aura.push((index, points))
            }
            _ => effect(map, caster, target, s, index, points, &here, item, t),
        }
        if e.chain_target > 1 {
            let falloff = l.mods.apply(
                mods::OP_EFFECT_PAST_FIRST,
                s.dmg_multiplier[index].max(0.0) * 100.0,
            ) / 100.0;
            chain[index] *= if falloff > 0.0 { falloff } else { 1.0 };
        }
    }
    if !weapon.is_empty() {
        weapon_damage(map, caster, target, s, &weapon, l);
    }
    if !pet_aura.is_empty() {
        crate::game::map::pets::area_aura(map, caster, s, &pet_aura);
    }
    if !aura_effects.is_empty() && map.unit_view(target).is_some_and(|u| u.alive) {
        let duration = aura_duration(map, caster, s, &l.mods, l.combo);
        // The target's returns before, should the aura not go on.
        let returns = map.caster(target).map(|c| c.diminishing.clone());
        let duration = if auras::positive(s) || target == caster {
            Some(duration)
        } else {
            diminished(map, caster, target, s, duration)
        };
        let Some(duration) = duration else {
            outcome::send_miss(map, caster, target, s.id, outcome::Hit::Immune);
            return;
        };
        // The caster's charge modifiers, whether the spell has charges or
        // not; none left is no limit (mangos's `SpellAuraHolder`, Thistle
        // Tea's `Application`).
        let charges = (s.proc_charges > 0 || l.mods.any(mods::OP_CHARGES))
            .then(|| l.mods.apply_ms(mods::OP_CHARGES, s.proc_charges));
        let ctx = auras::Context {
            item,
            equipped: false,
            combo: l.combo,
            duration: Some(duration),
            charges,
            triggered: l.triggered,
            linked: false,
            activation: (
                l.mods.flat[mods::OP_ACTIVATION_TIME],
                l.mods.pct[mods::OP_ACTIVATION_TIME],
            ),
        };
        if auras::apply_spell(map, caster, target, s, &aura_effects, ctx).is_err()
            && caster != target
        {
            outcome::send_miss(map, caster, target, s.id, outcome::Hit::Immune);
        }
        // An aura that didn't go on (a full bar refused it) spends none of
        // its target's returns, as Thistle Tea's refusal comes before them.
        let on = auras::auras(map, target).is_some_and(|a| {
            a.holders
                .iter()
                .any(|h| h.spell == s.id && h.caster == caster)
        });
        if !on
            && let Some(before) = returns
            && let Some(c) = map.caster_mut(target)
        {
            c.diminishing = before;
        }
        if !auras::positive(s) {
            let (done, taken) = outcome::proc_flags(s);
            crate::game::map::procs::trigger(
                map,
                crate::game::map::procs::Event {
                    actor: caster,
                    target,
                    actor_flags: done,
                    target_flags: taken,
                    ex: l.hit.proc_ex(),
                    spell: Some(s.id),
                    damage: 0,
                    school: s.school_mask(),
                    offhand: false,
                    ranged: s.dmg_class == 3,
                    extra_attack: false,
                },
            );
        }
        // An aura's threat: half a point a level for a helpful one, or the
        // spell's own for a harmful.
        if !auras::positive(s) {
            super::threat(map, caster, target, s, 0.0, &l.mods);
        }
    }
}

/// A weapon's speed for normalized damage: 1.7 for daggers, 3.3 for
/// two-handers, 2.8 for ranged, 2.4 for the rest.
fn normalized_speed(subclass: u32, ranged: bool) -> f32 {
    if ranged {
        return 2.8;
    }
    match subclass {
        15 => 1.7,
        1 | 5 | 6 | 8 | 10 => 3.3,
        _ => 2.4,
    }
}

/// A weapon blow: the weapon's roll (normalized if any effect is) plus the
/// flat effects, times the percent effects, then armor, the crit and the
/// block.
fn weapon_damage(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    effects: &[(usize, i32)],
    l: &Landing,
) {
    let data = map.realm.data.clone();
    let ranged = s.dmg_class == 3;
    let offhand = s.attr(3, sd::ATTR_EX3_REQ_OFFHAND);
    let normalized = effects
        .iter()
        .any(|(i, _)| s.effects[*i].effect == EFFECT_NORMALIZED_WEAPON_DMG);
    let (min, max) = map
        .unit_fields(caster)
        .map(|f| {
            if ranged {
                (
                    f.get_f32(UNIT_FIELD_MINRANGEDDAMAGE),
                    f.get_f32(UNIT_FIELD_MAXRANGEDDAMAGE),
                )
            } else if offhand {
                (
                    f.get_f32(UNIT_FIELD_MINOFFHANDDAMAGE),
                    f.get_f32(UNIT_FIELD_MAXOFFHANDDAMAGE),
                )
            } else {
                (
                    f.get_f32(UNIT_FIELD_MINDAMAGE),
                    f.get_f32(UNIT_FIELD_MAXDAMAGE),
                )
            }
        })
        .unwrap_or((1.0, 2.0));
    let mut base = roll_between(map, min, max);
    // Hunter's Mark: ranged attack power against its target; and melee
    // attack power against one so marked.
    {
        let (kind, period) = if ranged {
            (
                AURA_RANGED_ATTACK_POWER_ATTACKER_BONUS,
                UNIT_FIELD_RANGEDATTACKTIME,
            )
        } else if offhand {
            (
                AURA_MELEE_ATTACK_POWER_ATTACKER_BONUS,
                UNIT_FIELD_BASEATTACKTIME + 1,
            )
        } else {
            (
                AURA_MELEE_ATTACK_POWER_ATTACKER_BONUS,
                UNIT_FIELD_BASEATTACKTIME,
            )
        };
        let bonus = auras::auras(map, target).map_or(0, |a| a.sum(kind));
        let speed = map
            .unit_fields(caster)
            .map(|f| f.get_u32(period))
            .unwrap_or(2000);
        base += bonus.max(0) as f32 / 14.0 * speed as f32 / 1000.0;
    }
    if normalized && let Some(p) = map.players.get(&caster) {
        let g = crate::game::map::derive::gear(p, &data);
        let w = if ranged {
            g.ranged
        } else if offhand {
            g.off
        } else {
            g.main
        };
        if let Some(w) = w {
            let ap = if ranged {
                p.derived.ranged_attack_power
            } else {
                p.derived.attack_power
            } as f32;
            base =
                roll_between(map, w.min, w.max) + ap / 14.0 * normalized_speed(w.subclass, ranged);
        }
    }
    let flat: i32 = effects
        .iter()
        .filter(|(i, _)| s.effects[*i].effect != EFFECT_WEAPON_PERCENT_DAMAGE)
        .map(|(_, p)| *p)
        .sum();
    let pct: f32 = effects
        .iter()
        .filter(|(i, _)| s.effects[*i].effect == EFFECT_WEAPON_PERCENT_DAMAGE)
        .fold(1.0, |m, (_, p)| m * *p as f32 / 100.0);
    let index = effects[0].0;
    let mut weapon_spell = s.clone();
    weapon_spell.school = if effects
        .iter()
        .all(|(i, _)| s.effects[*i].effect == EFFECT_WEAPON_DAMAGE_NOSCHOOL)
    {
        0
    } else {
        s.school
    };
    let d = outcome::damage(
        map,
        caster,
        target,
        &weapon_spell,
        index,
        (base + flat as f32) * pct,
        l,
    );
    outcome::deal(map, caster, target, s, d, l);
}

fn roll_between(map: &Map, min: f32, max: f32) -> f32 {
    use rand::RngExt;
    if max <= min || !outcome::rolling(map) {
        return (min + max) / 2.0;
    }
    rand::rng().random_range(min..=max)
}

/// One effect on one target.
#[allow(clippy::too_many_arguments)]
fn effect(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    amount: i32,
    l: &Landing,
    item: Option<Guid>,
    t: &Targets,
) {
    let e = s.effects[index];
    match e.effect {
        // A beacon's planting is the battleground's: see `run_untargeted`.
        _ if crate::game::map::battleground::plants(s) => {}
        _ if crate::game::map::pets::handles(s, index) => {
            crate::game::map::pets::effect(map, caster, target, s, index, amount, l.triggered)
        }
        _ if crate::game::map::extra_effects::effect(
            map, caster, target, s, index, amount, item,
        ) => {}
        _ if crate::game::map::event_quests::effect(map, caster, target, s, index) => {}
        19 => {
            crate::game::map::combat::grant_extra_attacks(map, caster, s.id, amount.max(1) as u32);
            super::log::extra_attacks(map, caster, s.id, caster, amount.max(1) as u32);
        }
        // The ritual's summons: for its target, by its owner.
        crate::game::map::objects::EFFECT_SUMMON_PLAYER => {
            let who = t.unit.filter(|u| *u != caster).unwrap_or(target);
            if crate::game::guid::is_player(who) && who != caster {
                crate::game::map::misc::offer_summon(map, caster, who);
            }
        }
        EFFECT_ADD_HONOR if crate::game::guid::is_player(target) => {
            crate::game::map::pvp::bonus_honor(map, target, amount.max(0) as u32)
        }
        EFFECT_INSTAKILL => {
            super::class_scripts::sacrifice(map, caster, target, s);
            if target == caster {
                super::log::instakill(map, target, s.id);
            }
            let health = map
                .unit_fields(target)
                .map(|f| f.get_u32(UNIT_FIELD_HEALTH))
                .unwrap_or(0);
            combat::deal_damage_by(map, caster, target, health, 1, Some(s.id));
        }
        // The world's harm by a spell (a campfire's, a brazier's): its own
        // amount, no bonus and no crit. A player takes it as fire from the
        // world; anything else only has its shields soak it, and is told
        // (mangos's `EffectEnvironmentalDMG`, Thistle Tea's
        // `environmental_damage`).
        EFFECT_ENVIRONMENTAL_DAMAGE => {
            let amount = amount.max(0) as u32;
            if guid::is_player(target) {
                crate::game::map::environment::damage(
                    map,
                    target,
                    wow_world_messages::vanilla::EnvironmentalDamageType::Fire,
                    amount,
                );
            } else {
                let (_, absorbed) = auras::absorb(map, target, amount, s.school_mask());
                let frame = outcome::damage_log(
                    caster, target, s.id, s.school, amount, absorbed, 0, 0, false,
                );
                map.send_to_observers(target, &frame, true);
            }
        }
        EFFECT_SCHOOL_DAMAGE => {
            if super::scripts::school_damage(map, caster, target, s, index, amount, l) {
                return;
            }
            let base =
                super::class_scripts::damage_base(map, caster, s, index, amount.max(0) as f32, l);
            let mut d = outcome::damage(map, caster, target, s, index, base, l);
            let scale = super::class_scripts::damage_multiplier(map, target, s);
            if scale != 1.0 {
                d.amount = (d.amount as f32 * scale) as u32;
            }
            outcome::deal(map, caster, target, s, d, l);
        }
        EFFECT_HEALTH_LEECH => {
            let before = map
                .unit_fields(target)
                .map(|f| f.get_u32(UNIT_FIELD_HEALTH))
                .unwrap_or(0);
            let d = outcome::damage(map, caster, target, s, index, amount.max(0) as f32, l);
            outcome::deal(map, caster, target, s, d, l);
            let after = map
                .unit_fields(target)
                .map(|f| f.get_u32(UNIT_FIELD_HEALTH))
                .unwrap_or(0);
            let mult = if e.multiple_value > 0.0 {
                e.multiple_value
            } else {
                1.0
            };
            let taken = before.saturating_sub(after);
            heal(
                map,
                caster,
                caster,
                s.id,
                (taken as f32 * mult) as u32,
                false,
            );
        }
        EFFECT_HEAL | EFFECT_HEAL_MECHANICAL => {
            let amount = amount + super::class_scripts::swiftmend(map, target, s);
            let h = outcome::healing(map, caster, target, s, index, amount.max(0) as f32, l);
            heal_crit(map, caster, target, s.id, h, false, l.crit);
            crate::game::map::procs::trigger(
                map,
                crate::game::map::procs::Event {
                    actor: caster,
                    target,
                    actor_flags: crate::game::map::procs::PROC_DONE_POSITIVE_SPELL,
                    target_flags: crate::game::map::procs::PROC_TAKEN_POSITIVE_SPELL,
                    ex: if l.crit {
                        crate::game::map::procs::EX_CRITICAL_HIT
                    } else {
                        crate::game::map::procs::EX_NORMAL_HIT
                    },
                    spell: Some(s.id),
                    damage: h,
                    school: s.school_mask(),
                    offhand: false,
                    ranged: s.dmg_class == 3,
                    extra_attack: false,
                },
            );
        }
        EFFECT_HEAL_MAX_HEALTH => {
            let max = map
                .unit_fields(caster)
                .map(|f| f.get_u32(UNIT_FIELD_MAXHEALTH))
                .unwrap_or(0);
            // Lay on Hands takes only the percentages, as Thistle Tea's
            // `heal_max_health` does.
            let h = outcome::healing_taken(map, target, max as f32) as u32;
            heal(map, caster, target, s.id, h, false);
        }
        // Into any pool the target has, shown or not, alive, of a power
        // that is one and never less (mangos's `EffectEnergize`, Thistle
        // Tea's `PowerRestoration`); logged for the whole amount.
        EFFECT_ENERGIZE => {
            if !(0..5).contains(&e.misc_value)
                || amount < 0
                || !map.unit_view(target).is_some_and(|u| u.alive)
            {
                return;
            }
            let power = e.misc_value as u16;
            if map
                .unit_fields(target)
                .is_none_or(|f| f.get_u32(UNIT_FIELD_MAXPOWER1 + power) == 0)
            {
                return;
            }
            if let Some(f) = map.unit_fields_mut(target) {
                let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
                let now = f.get_u32(UNIT_FIELD_POWER1 + power);
                f.set_u32(
                    UNIT_FIELD_POWER1 + power,
                    (now + amount.max(0) as u32).min(max),
                );
            }
            if let Some(frame) = crate::game::conn::encode(&SMSG_SPELLENERGIZELOG {
                victim: target,
                caster,
                spell: s.id,
                power: Power::try_from(power as u8).unwrap_or(Power::Mana),
                damage: amount.max(0) as u32,
            }) {
                map.send_to_observers(target, &frame, true);
            }
            super::scripts::after_energize(map, caster, s);
        }
        // A pool the target has shown, alive, and a power that is one
        // (mangos's `EffectPowerDrain` and `EffectPowerBurn`, Thistle Tea's
        // `PowerLeech` and `PowerBurn`): a druid in cat form keeps their
        // mana from a Mana Burn.
        EFFECT_POWER_DRAIN | EFFECT_POWER_BURN => {
            if !(0..5).contains(&e.misc_value) || !map.unit_view(target).is_some_and(|u| u.alive) {
                return;
            }
            let power = e.misc_value as u16;
            let shown = shown_power(map, target);
            if shown != power
                && !(e.effect == EFFECT_POWER_DRAIN
                    && power == crate::game::map::pets::POWER_HAPPINESS)
            {
                return;
            }
            let taken = map
                .unit_fields_mut(target)
                .map(|f| {
                    let have = f.get_u32(UNIT_FIELD_POWER1 + power);
                    let t = (amount.max(0) as u32).min(have);
                    f.set_u32(UNIT_FIELD_POWER1 + power, have - t);
                    t
                })
                .unwrap_or(0);
            if e.effect == EFFECT_POWER_DRAIN {
                // Nothing back from oneself, nor of a power one doesn't
                // use; a multiplier of none is one (Thistle Tea's).
                let mult = if caster == target || shown_power(map, caster) != power {
                    0.0
                } else {
                    let base = if e.multiple_value > 0.0 {
                        e.multiple_value
                    } else {
                        1.0
                    };
                    l.mods.apply(super::mods::OP_MULTIPLE_VALUE, base).max(0.0)
                };
                super::log::power_drain(map, caster, s.id, target, taken, power, mult);
                if let Some(f) = map.unit_fields_mut(caster) {
                    let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
                    let now = f.get_u32(UNIT_FIELD_POWER1 + power);
                    f.set_u32(
                        UNIT_FIELD_POWER1 + power,
                        (now + (taken as f32 * mult) as u32).min(max),
                    );
                }
            } else {
                // What is burned is damage, by the multiplier as the
                // caster's modifiers have it (none is none: Chains of Ice
                // burns without harm), a crit's half more and no spell
                // power.
                let mult = l
                    .mods
                    .apply(super::mods::OP_MULTIPLE_VALUE, e.multiple_value)
                    .max(0.0);
                let mut damage = taken as f32 * mult;
                if l.crit {
                    let pct = l.mods.pct[super::mods::OP_CRIT_DAMAGE_BONUS] as f32 / 100.0;
                    damage += damage * 0.5 * (1.0 + pct);
                }
                let d = outcome::Damage {
                    amount: damage as u32,
                    resisted: 0,
                    blocked: 0,
                };
                outcome::deal(map, caster, target, s, d, l);
            }
        }
        EFFECT_APPLY_AURA | EFFECT_APPLY_AREA_AURA_PARTY => {}
        EFFECT_TELEPORT_UNITS => {
            let data = map.realm.data.clone();
            let to = match data.spells.target_positions.get(&s.id) {
                Some(&(m, x, y, z, o)) => Some((m, Vector3d { x, y, z }, o)),
                // The hearthstone and its kind: home.
                None => home(map, target),
            };
            if let Some((m, pos, o)) = to {
                map.teleport(target, crate::game::map::MapKey::world(m), pos, o);
            }
        }
        // To the caster, as mangos's `EffectTeleUnitsFaceCaster` and
        // Thistle Tea's `TeleportNearCaster` have it: the spell's place in
        // `spell_target_position` on this map (a database location, 17),
        // the caster's own spot (18), or ahead of the caster by the
        // effect's radius (47, the front), past both their sizes else;
        // turned to the caster's facing negated.
        EFFECT_TELEPORT_UNITS_FACE_CASTER => {
            if let Some(me) = map.unit_view(caster) {
                let (from, o) = (me.position, me.orientation);
                let size = |map: &Map, g: Guid| {
                    map.unit_fields(g)
                        .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS).max(0.0))
                };
                let radius = map.realm.data.spells.radius(e.radius_index);
                let ahead = |d: f32| Vector3d {
                    x: from.x + d * o.cos(),
                    y: from.y + d * o.sin(),
                    z: from.z,
                };
                let selectors = [e.target_a, e.target_b];
                let stored = map
                    .realm
                    .data
                    .spells
                    .target_positions
                    .get(&s.id)
                    .filter(|(m, ..)| *m == map.key.map)
                    .map(|&(_, x, y, z, _)| Vector3d { x, y, z });
                let at = if selectors.contains(&17)
                    && let Some(at) = stored
                {
                    at
                } else if selectors.contains(&18) {
                    from
                } else if selectors.contains(&47) && !selectors.contains(&17) {
                    first_clear(map, from, ahead(radius))
                } else {
                    let d = radius + size(map, target) + size(map, caster);
                    first_clear(map, from, ahead(d))
                };
                let facing = (-o).rem_euclid(std::f32::consts::TAU);
                map.teleport(target, map.key, at, facing);
            }
        }
        // Back to where it last stood on the ground (Thistle Tea's
        // `SafePosition`), not home; none known, nowhere.
        EFFECT_STUCK => {
            let here = map.key.map;
            if let Some((_, pos, o)) = map
                .players
                .get(&target)
                .and_then(|p| p.safe_position)
                .filter(|(m, _, _)| *m == here)
            {
                crate::game::map::movement::teleport_within(map, target, pos, o);
            }
        }
        EFFECT_BIND => {
            let to = lands_on(map, caster, target, &e, t);
            crate::game::map::npc::bind_home(map, to, caster);
        }
        // A leap: a player's client is sent there, a creature put there;
        // not one on a flight (mangos's `EffectLeap`, Thistle Tea's `Leap`).
        EFFECT_LEAP => {
            let flying = map.players.get(&caster).is_some_and(|p| p.flight.is_some());
            if let Some(me) = map.unit_view(caster).filter(|_| !flying) {
                let d = super::targets::radius(map, s, index, &l.mods).max(10.0);
                let o = me.orientation + leap_angle(e.target_a, e.target_b);
                let want = Vector3d {
                    x: me.position.x + d * o.cos(),
                    y: me.position.y + d * o.sin(),
                    z: me.position.z,
                };
                let to = first_clear(map, me.position, want);
                if map.players.contains_key(&caster) {
                    map.teleport(caster, map.key, to, me.orientation);
                } else {
                    super::super::scripts::place(map, caster, to, me.orientation);
                }
            }
        }
        // A run to the target, and a swing there if it is a harmful spell
        // that doesn't end the fight (Thistle Tea's `attack_on_arrival?`).
        EFFECT_CHARGE => {
            let attack = !auras::positive(s)
                && !s.attr(0, crate::game::data::spells::ATTR_CANCELS_AUTO_ATTACK);
            crate::game::map::charge::start(map, caster, target, attack);
        }
        EFFECT_KNOCK_BACK => knock_back(map, caster, target, e.misc_value, amount),
        EFFECT_CREATE_ITEM if e.item_type != 0 => {
            if let Some(to) = created_for(map, caster, target, &e, t) {
                let item = super::scripts::created_item(
                    map,
                    caster,
                    s,
                    u32::try_from(e.item_type).unwrap_or(0),
                );
                // A player's own make is created; another's gift received.
                // What doesn't fit, of room or of a unique allowance, isn't
                // made (mangos's `DoCreateItem`, Thistle Tea's clamp).
                let count = map
                    .players
                    .get(&to)
                    .and_then(|p| {
                        crate::game::map::inventory::fitting(
                            p,
                            &map.realm.data,
                            item,
                            amount.max(1) as u32,
                        )
                        .ok()
                    })
                    .unwrap_or(amount.max(1) as u32);
                let made = if to == caster {
                    crate::game::map::inventory::add_created_item(map, to, item, count)
                } else {
                    crate::game::map::inventory::add_item(map, to, item, count, true)
                };
                // A recipe's skill rises once its product is stored (Thistle
                // Tea's `commit_store`, mangos's `UpdateCraftSkill`).
                if made.is_ok() {
                    crate::game::map::professions::recipe_skill_up(map, caster, s);
                }
            }
        }
        EFFECT_LEARN_SPELL if e.trigger_spell != 0 => learn(map, target, e.trigger_spell),
        EFFECT_TRIGGER_SPELL if e.trigger_spell != 0 => {
            cast_triggered(map, caster, e.trigger_spell, Some(target));
        }
        EFFECT_DISPEL => {
            dispel(map, caster, target, s, e.misc_value, amount.max(1) as u32);
        }
        EFFECT_DISPEL_MECHANIC => {
            let mechanic = e.misc_value.max(0) as u32;
            let data = map.realm.data.clone();
            auras::remove_where(map, target, |h| {
                !h.passive
                    && (h.mechanic == mechanic
                        || data.spells.get(h.spell).is_some_and(|t| {
                            t.effects
                                .iter()
                                .any(|x| x.mechanic == mechanic && x.effect != 0)
                        }))
            });
        }
        EFFECT_INTERRUPT_CAST => {
            // A cast a silence could stop is interrupted (mangos's
            // `IsInterruptible`: its interrupt flag 0x8 and prevention 1),
            // not the Hearthstone; its school locked out for the spell's
            // time.
            let interrupted = map
                .caster(target)
                .and_then(|c| c.casting.as_ref())
                .map(|c| c.spell)
                .filter(|id| {
                    map.realm
                        .data
                        .spells
                        .get(*id)
                        .is_some_and(|t| t.interrupt_flags & 0x8 != 0 && t.prevention_type == 1)
                });
            if interrupted.is_some() {
                super::interrupt(map, target);
            }
            if let Some(spell) = interrupted {
                super::log::interrupt(map, caster, s.id, target, spell);
                let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
                let ms = map.realm.data.spells.duration(s, level).unwrap_or(0);
                if ms > 0 {
                    super::cooldowns::lock_school(map, target, spell, ms);
                }
            }
        }
        EFFECT_THREAT => {
            let a = l.mods.apply(mods::OP_THREAT, amount as f32);
            crate::game::map::ai::add_threat(map, target, caster, a);
        }
        EFFECT_MODIFY_THREAT_PERCENT => {
            if let Some(c) = map.creature_mut(target) {
                for (g, t) in c.threat.iter_mut() {
                    if *g == caster {
                        *t = (*t * (100 + amount) as f32 / 100.0).max(0.0);
                    }
                }
            }
        }
        EFFECT_ATTACK_ME => {
            if guid::is_unit(target) {
                super::threat_sync::matched(map, target, caster);
                crate::game::map::ai::taunted(map, target, caster);
            }
        }
        EFFECT_ATTACK => combat::start_attack(map, caster, target),
        EFFECT_SANCTUARY => {
            // Vanish: nothing fights the caster any more.
            combat::leave_combat(map, target);
            crate::game::map::ai::forget(map, target);
            let snared: Vec<u32> = auras::auras(map, target)
                .map(|a| {
                    a.holders
                        .iter()
                        .filter(|h| {
                            !h.positive && (h.has(AURA_MOD_ROOT) || h.has(AURA_MOD_DECREASE_SPEED))
                        })
                        .map(|h| h.spell)
                        .collect()
                })
                .unwrap_or_default();
            for spell in snared {
                auras::remove_spell(map, target, spell, None);
            }
        }
        // Its amount is seconds.
        EFFECT_DISTRACT => {
            if let Some(at) = t.dest {
                crate::game::map::creature_motion::distract(map, target, at, amount);
            }
        }
        EFFECT_ADD_COMBO_POINTS => {
            add_combo_points(map, caster, target, amount.max(1) as u8);
            retain_combo_points(map, caster, s, l);
        }
        EFFECT_DUEL => crate::game::map::duel::challenge(map, caster, target),
        EFFECT_RESURRECT | EFFECT_RESURRECT_NEW => {
            // A player's name the client knows; a creature's is sent.
            let (name, spirit_healer) = map.creature(caster).map_or((String::new(), false), |c| {
                let name = map
                    .realm
                    .data
                    .creature_templates
                    .get(&c.entry)
                    .map(|t| t.name.to_string())
                    .unwrap_or_default();
                let flags = c.fields.get_u32(UNIT_NPC_FLAGS);
                (
                    name,
                    flags & crate::game::map::npc::NPC_FLAG_SPIRITHEALER != 0,
                )
            });
            if let Some(p) = map.players.get_mut(&target)
                && p.death != crate::game::map::death::Death::Alive
                && p.resurrect_offer.is_none()
            {
                // `RESURRECT` gives a share of health and mana; the newer
                // effect, health and the misc value's mana (Thistle Tea's,
                // mangos's).
                let (health, mana) = if e.effect == EFFECT_RESURRECT_NEW {
                    (amount.max(1) as u32, e.misc_value.max(0) as u32)
                } else {
                    let share = amount.max(0) as f32 / 100.0;
                    (
                        ((p.fields.get_u32(UNIT_FIELD_MAXHEALTH) as f32 * share) as u32).max(1),
                        (p.fields.get_u32(UNIT_FIELD_MAXPOWER1) as f32 * share) as u32,
                    )
                };
                p.resurrect_offer = Some((caster, health, mana));
                p.outbound.send_frame(resurrect_request(
                    caster,
                    &name,
                    spirit_healer,
                    !s.attr(3, sd::ATTR_EX3_NO_RES_TIMER),
                ));
            }
        }
        EFFECT_SKILL | EFFECT_SKILL_STEP => {
            let skill = e.misc_value.max(0) as u32;
            let step = amount.max(1) as u32;
            if let Some(p) = map.players.get_mut(&target) {
                crate::game::map::skills::learn_skill(p, skill, step);
            }
        }
        EFFECT_WEAPON | EFFECT_PROFICIENCY => proficiency(map, target, s),
        EFFECT_QUEST_COMPLETE => {
            let quest = e.misc_value.max(0) as u32;
            // One aimed at a creature (a rod's tame at its beast) is the
            // caster's.
            let target = if map.players.contains_key(&target) {
                target
            } else {
                caster
            };
            if let Some(p) = map.players.get_mut(&target)
                && let Some(slot) = p.journal.slot_of(quest)
            {
                p.journal.log[slot].explored = true;
                crate::game::map::quests::check(map, target, quest);
            }
        }
        EFFECT_DURABILITY_DAMAGE | EFFECT_DURABILITY_DAMAGE_PCT => {
            let data = map.realm.data.clone();
            let pct = e.effect == EFFECT_DURABILITY_DAMAGE_PCT;
            let slot = e.misc_value;
            // -1 is what is worn, -2 that and what is carried in the bags
            // (not the bank), a slot up to the last bag's one item
            // (mangos's `DurabilityPointsLossAll`, Thistle Tea's
            // `spell_scope`).
            let scoped = |i: &crate::game::map::player::Item| match slot {
                -1 => i.bag == crate::game::store::BAG_PLAYER && i.slot < 19,
                -2 => {
                    i.carried()
                        && !(i.bag == crate::game::store::BAG_PLAYER && (19..23).contains(&i.slot))
                }
                s if (0..23).contains(&s) => {
                    i.bag == crate::game::store::BAG_PLAYER && i32::from(i.slot) == s
                }
                _ => false,
            };
            // The combat log names the item a slot's loss is from; a loss
            // to everything is all items' -1, as mangos's
            // EffectDurabilityDamage and Thistle Tea log it.
            let logged = if slot < 0 {
                map.players.contains_key(&target).then_some(u32::MAX)
            } else {
                map.players.get(&target).and_then(|p| {
                    p.items
                        .iter()
                        .find(|i| scoped(i) && i.fields.get_u32(ITEM_FIELD_MAXDURABILITY) > 0)
                        .map(|i| i.entry)
                })
            };
            if !pct && let Some(entry) = logged {
                super::log::durability(map, caster, s.id, target, entry);
            }
            if let Some(p) = map.players.get_mut(&target) {
                let mut turned = false;
                for it in p.items.iter_mut().filter(|i| scoped(i)) {
                    let max = it.fields.get_u32(ITEM_FIELD_MAXDURABILITY);
                    let now = it.fields.get_u32(ITEM_FIELD_DURABILITY);
                    let then = crate::game::map::vendor::worn(max, now, amount, pct);
                    it.fields.set_u32(ITEM_FIELD_DURABILITY, then);
                    turned |= max > 0 && (now == 0) != (then == 0);
                }
                // Gear broken or mended gives what it gives no more, or
                // again.
                if turned {
                    p.apply_stats(&data);
                }
                p.gear_dirty = true;
            }
        }
        EFFECT_DUMMY | EFFECT_SCRIPT_EFFECT
            if !super::class_scripts::dummy(map, caster, target, s, index, amount)
                && !super::holiday::effect(map, caster, target, s)
                && !crate::game::map::boss_mechanics::dummy(map, caster, target, s)
                && !crate::game::map::outdoor_pvp::carrier_pulse(map, target, s) =>
        {
            super::scripts::dummy(map, caster, target, s, index, amount, l, item);
        }
        _ => {}
    }
}

/// A dispel of `kind` with `count` tries, as Thistle Tea's `Dispel` makes
/// it: each try takes one stack of one of the auras it may take (picked at
/// random), unless that aura's caster's talents hold it on; magic and
/// poison only of the side the target is on to the dispeller, curses and
/// diseases either way. A passive aura goes like any other, as in Thistle
/// Tea and mangos (a creature's Fire Shield): its dispel type decides.
fn dispel(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate, kind: i32, count: u32) {
    // An enemy's buffs, a friend's afflictions.
    let positive = combat::can_attack(map, caster, target);
    let matches = |dispel: u32, holder_positive: bool| {
        let kind_ok = if kind == 7 || kind < 0 {
            matches!(dispel, 1..=4)
        } else {
            dispel == kind as u32
        };
        kind_ok && (!matches!(dispel, 1 | 4) || holder_positive == positive)
    };
    let mut candidates: Vec<(u32, Guid, u32)> = auras::auras(map, target)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| h.dispel != 0 && matches(h.dispel, h.positive))
                .map(|h| (h.spell, h.caster, h.stacks.max(1)))
                .collect()
        })
        .unwrap_or_default();
    let data = map.realm.data.clone();
    let mut taken: Vec<(u32, Guid, u32)> = Vec::new();
    let mut held = Vec::new();
    for _ in 0..count {
        if candidates.is_empty() {
            break;
        }
        let i = if outcome::rolling(map) {
            use rand::RngExt;
            rand::rng().random_range(0..candidates.len())
        } else {
            0
        };
        let (spell, by, stacks) = candidates[i];
        if stacks > 1 {
            candidates[i].2 -= 1;
        } else {
            candidates.remove(i);
        }
        // An aura its caster's talents hold on may resist (Thistle Tea's
        // `DispelResistance`).
        let resist = data.spells.get(spell).map_or(0, |t| {
            super::mods::for_spell(map, by, t).flat[super::mods::OP_RESIST_DISPEL_CHANCE]
        });
        if resist > 0 && outcome::chance(map, resist as f32) {
            held.push(spell);
            continue;
        }
        match taken.iter_mut().find(|(t, c, _)| *t == spell && *c == by) {
            Some(t) => t.2 += 1,
            None => taken.push((spell, by, 1)),
        }
    }
    let ids: Vec<u32> = taken.iter().map(|(spell, _, _)| *spell).collect();
    for (spell, by, n) in &taken {
        auras::remove_stacks(map, target, *spell, *by, *n);
    }
    super::log::dispelled(map, caster, target, ids);
    super::log::dispel_failed(map, caster, target, held);
    if !taken.is_empty() {
        super::class_scripts::dispelled(map, caster, s);
    }
}

/// Who a create-item effect gives its item to, as Thistle Tea and mangos
/// give it: the player it lands on ([`lands_on`]: an innkeeper's Bind gives
/// the one it binds the Hearthstone). One landing on a creature gives the
/// caster the item only if it names the caster too (an Empty Phial filled
/// at one).
fn created_for(
    map: &Map,
    caster: Guid,
    target: Guid,
    e: &crate::game::data::spells::SpellEffect,
    t: &Targets,
) -> Option<Guid> {
    let to = lands_on(map, caster, target, e, t);
    let to = if !map.players.contains_key(&to) && (e.target_a == 1 || e.target_b == 1) {
        caster
    } else {
        to
    };
    map.players.contains_key(&to).then_some(to)
}

/// Whom an effect lands on for the effects that follow Thistle Tea's
/// targeting where wvs's differs: one naming no target, cast by a creature
/// at a unit, lands on that unit rather than the caster.
fn lands_on(
    map: &Map,
    caster: Guid,
    target: Guid,
    e: &crate::game::data::spells::SpellEffect,
    t: &Targets,
) -> Guid {
    if e.target_a == 0 && e.target_b == 0 && !map.players.contains_key(&caster) {
        t.unit.unwrap_or(target)
    } else {
        target
    }
}

/// Where a player's hearthstone goes.
/// The way a leap goes off its caster's facing, by its implicit targets
/// (mangos's caster-relative targets, Thistle Tea's `CasterLocation`):
/// back (48), left (49), right (50), the four between (41 to 44); ahead
/// for the rest (Blink's 55, 47).
fn leap_angle(target_a: u32, target_b: u32) -> f32 {
    use std::f32::consts::PI;
    [target_b, target_a]
        .iter()
        .find_map(|t| match t {
            48 => Some(PI),
            49 => Some(PI / 2.0),
            50 => Some(-PI / 2.0),
            41 => Some(1.75 * PI),
            42 => Some(1.25 * PI),
            43 => Some(0.75 * PI),
            44 => Some(0.25 * PI),
            _ => None,
        })
        .unwrap_or(0.0)
}

/// Where a leap from `from` toward `to` ends, on the ground: `to` if it can
/// be seen, else the farthest point toward it that can, found by halving
/// the way ten times as Thistle Tea's `first_collision_position` does, and
/// half a yard back from what stopped it (its `collision_position`).
fn first_clear(map: &mut Map, from: Vector3d, to: Vector3d) -> Vector3d {
    let ground = |map: &mut Map, p: Vector3d| {
        let z = map.nav.as_mut().and_then(|n| n.ground(p.x, p.y, p.z + 2.0));
        Vector3d {
            z: z.unwrap_or(p.z),
            ..p
        }
    };
    let along = |f: f32| Vector3d {
        x: from.x + (to.x - from.x) * f,
        y: from.y + (to.y - from.y) * f,
        z: from.z + (to.z - from.z) * f,
    };
    let end = ground(map, to);
    if super::checks::line_of_sight(map, from, end) {
        return end;
    }
    let (mut clear, mut blocked) = (0.0f32, 1.0f32);
    for _ in 0..10 {
        let f = (clear + blocked) / 2.0;
        let at = ground(map, along(f));
        if super::checks::line_of_sight(map, from, at) {
            clear = f;
        } else {
            blocked = f;
        }
    }
    let length = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();
    ground(map, along((clear - 0.5 / length.max(0.001)).max(0.0)))
}

/// `SMSG_RESURRECT_REQUEST`, as mangos-zero lays it out: the caster, its
/// name (none for a player), whether it is a spirit healer, and whether
/// the client holds the acceptance back by its resurrection timer.
fn resurrect_request(
    caster: Guid,
    name: &str,
    spirit_healer: bool,
    timer: bool,
) -> std::sync::Arc<[u8]> {
    let mut b = Body::default();
    b.u64(caster.guid())
        .u32(name.len() as u32 + 1)
        .cstr(name)
        .u8(spirit_healer.into())
        .u8(timer.into());
    b.frame(SMSG_RESURRECT_REQUEST)
}

const SMSG_RESURRECT_REQUEST: u16 = 0x15B;

fn home(map: &Map, g: Guid) -> Option<(u32, Vector3d, f32)> {
    map.players.get(&g).and_then(|p| p.row.state.bind).map(|b| {
        (
            b.map,
            Vector3d {
                x: b.x,
                y: b.y,
                z: b.z,
            },
            0.0,
        )
    })
}

/// Combo points on `target`: a new target starts them over. Points held
/// only for a while (`retain_combo_points`) are kept for good.
pub fn add_combo_points(map: &mut Map, caster: Guid, target: Guid, n: u8) {
    release_retained_combo_points(map, caster);
    if let Some(c) = map.caster_mut(caster) {
        if c.combo_target != target {
            c.combo_target = target;
            c.combo_points = 0;
        }
        c.combo_points = (c.combo_points + n).min(5);
        let points = c.combo_points;
        if let Some(p) = map.players.get_mut(&caster) {
            p.fields.set_guid(PLAYER_FIELD_COMBO_TARGET, target);
            p.fields.set_byte(PLAYER_FIELD_BYTES, 1, points);
        }
    }
}

/// Whether `s` gives combo points.
fn awards_combo_points(s: &SpellTemplate) -> bool {
    s.effects
        .iter()
        .any(|e| e.effect == EFFECT_ADD_COMBO_POINTS)
}

/// A builder whose points are held for a while (Premeditation's
/// `retain_combo_points`, aura 148): the hold goes on its player with the
/// points, as Thistle Tea's `ComboPoints.award` puts it; when it runs out
/// its points come off again (`retained_combo_points_expired`).
fn retain_combo_points(map: &mut Map, caster: Guid, s: &SpellTemplate, l: &Landing) {
    if !map.players.contains_key(&caster) {
        return;
    }
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    let held: Vec<(usize, i32)> = (0..3)
        .filter(|i| {
            s.effects[*i].effect == EFFECT_APPLY_AURA
                && s.effects[*i].aura == AURA_RETAIN_COMBO_POINTS
        })
        .map(|i| (i, l.mods.points(s, i, points(s, i, level))))
        .collect();
    if held.is_empty() {
        return;
    }
    let ctx = auras::Context {
        duration: Some(aura_duration(map, caster, s, &l.mods, 0)),
        ..Default::default()
    };
    let _ = auras::apply_spell(map, caster, caster, s, &held, ctx);
}

/// A builder or a finisher makes points held for a while the player's for
/// good, or spends them: the holds come off, taking nothing back.
pub fn release_retained_combo_points(map: &mut Map, caster: Guid) {
    let held = auras::auras(map, caster)
        .is_some_and(|a| a.holders.iter().any(|h| h.has(AURA_RETAIN_COMBO_POINTS)));
    if held {
        auras::remove_where(map, caster, |h| h.has(AURA_RETAIN_COMBO_POINTS));
    }
}

/// A hold on combo points ran out (Thistle Tea's `ComboPoints.expire`):
/// its points come off those on the target, if there still is one.
pub fn retained_combo_points_expired(map: &mut Map, g: Guid, h: &auras::Holder) {
    let amount: i32 = h
        .effects
        .iter()
        .filter(|x| x.kind == AURA_RETAIN_COMBO_POINTS)
        .map(|x| x.amount.max(0))
        .sum();
    if amount == 0 || !map.players.contains_key(&g) {
        return;
    }
    let Some(c) = map.caster_mut(g) else {
        return;
    };
    if c.combo_target.is_zero() {
        return;
    }
    c.combo_points = c.combo_points.saturating_sub(amount.min(5) as u8);
    let points = c.combo_points;
    if let Some(p) = map.players.get_mut(&g) {
        p.fields.set_byte(PLAYER_FIELD_BYTES, 1, points);
    }
}

/// A knock back: a player is thrown by their client; a creature slides.
/// Whether a knockback can move `g` (Thistle Tea's `Knockback.movable?`):
/// alive, not on a walk the server drives (a charge, fear or confusion, a
/// creature's walk under way), rooted, stunned, nor on a flight.
fn knockable(map: &Map, g: Guid) -> bool {
    let held = auras::auras(map, g).is_some_and(|a| {
        a.has(AURA_MOD_ROOT)
            || a.has(AURA_MOD_STUN)
            || a.has(AURA_MOD_FEAR)
            || a.has(AURA_MOD_CONFUSE)
    });
    let free = match map.players.get(&g) {
        Some(p) => p.flight.is_none() && p.driven.is_none() && p.charge.is_none(),
        None => map.creature(g).is_some_and(|c| {
            c.engagement.control.is_none()
                && !matches!(&c.motion, crate::game::map::creature::Motion::Moving(w) if !w.done(map.now))
        }),
    };
    !held && free && map.unit_view(g).is_some_and(|u| u.alive)
}

/// A knockback, as Thistle Tea's `Knockback.launch` has it: a unit free to
/// move has its cast interrupted, and the client that moves it (a player's,
/// or a possessed creature's master's) is told to launch it. A creature the
/// server runs stays where it is (mangos: "no non player controlled
/// pre-wotlk").
fn knock_back(map: &mut Map, caster: Guid, target: Guid, horizontal: i32, vertical: i32) {
    let (Some(me), Some(them)) = (map.unit_view(caster), map.unit_view(target)) else {
        return;
    };
    if !knockable(map, target) {
        return;
    }
    let angle = if caster == target {
        me.orientation + std::f32::consts::PI
    } else {
        (them.position.y - me.position.y).atan2(them.position.x - me.position.x)
    };
    let speed_xy = horizontal as f32 / 10.0;
    let speed_z = -(vertical as f32) / 10.0;
    super::interrupt(map, target);
    let mover = if map.players.contains_key(&target) {
        Some(target)
    } else {
        crate::game::map::pets::state(map, target)
            .filter(|s| s.kind == crate::game::map::pets::Kind::Possessed)
            .map(|s| s.owner)
    };
    // A launch begins a fall of its own: what came before it is gone
    // (Thistle Tea's `Knockback` resets the fall).
    if let Some(p) = map.players.get_mut(&target) {
        p.fall_from = None;
    }
    if let Some(p) = mover.and_then(|g| map.players.get(&g)) {
        let mut b = crate::game::data::items::Body::default();
        crate::game::update::write_packed_guid(&mut b.0, target);
        b.u32(0)
            .f32(angle.cos())
            .f32(angle.sin())
            .f32(speed_xy)
            .f32(speed_z);
        p.outbound.send_frame(b.frame(0x0EF));
    }
}

/// `SMSG_SET_PROFICIENCY`: the weapon or armor kinds a learned skill
/// allows, as equipping has them (`inventory::Proficiency`, Thistle Tea's
/// `send_proficiencies`).
pub fn proficiency(map: &Map, player: Guid, s: &SpellTemplate) {
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let prof = crate::game::map::inventory::Proficiency::of(p, &map.realm.data);
    let (class, mask) = match s.equipped_item_class {
        2 => (2, prof.weapons),
        4 => (4, prof.armor),
        _ => return,
    };
    let mut b = crate::game::data::items::Body::default();
    b.u8(class as u8).u32(mask);
    p.outbound.send_frame(b.frame(0x127));
}

/// The effects with no unit: a persistent area at the destination, a
/// missile's spell there, an object to open.
#[allow(clippy::too_many_arguments)]
pub fn run_untargeted(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    m: &Mods,
    combo: u8,
    // The effects the professions ran (opening locks, gathering): skipped.
    done: u8,
    triggered: bool,
) {
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    for index in 0..3 {
        if done & (1 << index) != 0 {
            continue;
        }
        let e = s.effects[index];
        match e.effect {
            _ if crate::game::map::battleground::plants(s) => {
                if e.effect == crate::game::map::professions::EFFECT_TRANS_DOOR {
                    crate::game::map::battleground::plant(map, caster, e.misc_value.max(0) as u32);
                }
            }
            // Objects summoned and worked: portals, campfires, traps,
            // rituals.
            _ if crate::game::map::objects::effect(map, caster, s, index, t) => {}
            crate::game::map::battleground::insignia::EFFECT_SKIN_PLAYER_CORPSE => {
                crate::game::map::battleground::insignia::remove(map, caster, t.object.or(t.unit));
            }
            EFFECT_PERSISTENT_AREA_AURA => {
                let at = t
                    .dest
                    .or_else(|| t.unit.and_then(|u| map.position_of(u)))
                    .or_else(|| map.position_of(caster));
                let Some(at) = at else {
                    continue;
                };
                let radius = super::targets::radius(map, s, index, m).max(1.0);
                let ms = aura_duration(map, caster, s, m, combo).unwrap_or(0);
                let period = Duration::from_millis(u64::from(e.amplitude.max(1000)));
                let l = Landing {
                    mods: m.clone(),
                    ..Default::default()
                };
                let base = m.points(s, index, points_with(s, index, level, combo));
                let amount = aura_amount(map, caster, caster, s, index, base, &l);
                let now = map.now;
                let object = super::dynamic::spawn(
                    map,
                    caster,
                    s.id,
                    super::dynamic::KIND_AREA_SPELL,
                    at,
                    radius,
                    Some(ms),
                );
                map.spells.areas.push(Area {
                    object,
                    caster,
                    spell: s.id,
                    index,
                    amount,
                    at,
                    radius,
                    ends: now + Duration::from_millis(u64::from(ms)),
                    // A periodic one's first tick a period in (mangos's
                    // PersistentAreaAura); a slow on those inside at once.
                    next_tick: now
                        + if s.channeled() || periodic_area(e.aura) {
                            period
                        } else {
                            Duration::ZERO
                        },
                    period,
                });
            }
            EFFECT_ADD_FARSIGHT => {
                let ms = aura_duration(map, caster, s, m, combo);
                super::sight::far_sight(map, caster, s, index, t, ms);
            }
            EFFECT_TRIGGER_MISSILE if e.trigger_spell != 0 => {
                if let Some(at) = t.dest {
                    cast_triggered_at(map, caster, e.trigger_spell, at);
                } else {
                    cast_triggered(map, caster, e.trigger_spell, t.unit);
                }
            }
            EFFECT_OPEN_LOCK => {
                if let Some(object) = t.object {
                    crate::game::map::loot::open_object(map, caster, object);
                }
            }
            // Summons and totems aim at a place: the caster's side.
            _ if crate::game::map::pets::handles(s, index)
                && super::targets::implicit(e.target_a) == super::targets::Implicit::Location =>
            {
                let amount = m.points(s, index, points_with(s, index, level, combo));
                crate::game::map::pets::effect(map, caster, caster, s, index, amount, triggered);
            }
            _ => {}
        }
    }
}

/// The persistent areas tick: their aura's effect on every enemy (or, for
/// a helpful one, friend) inside.
pub fn tick_areas(map: &mut Map) {
    let now = map.now;
    map.spells
        .area_ticked
        .retain(|_, t| now.saturating_duration_since(*t) < Duration::from_secs(30));
    // The tick due as an area ends is had before it goes (mangos's
    // `Aura::Update` keeps the last tick): Blizzard's eighth wave.
    let due: Vec<usize> = (0..map.spells.areas.len())
        .filter(|i| {
            let a = &map.spells.areas[*i];
            a.next_tick <= now && (a.next_tick <= a.ends || a.ends > now)
        })
        .collect();
    let data = map.realm.data.clone();
    for i in due {
        let Some(area) = map.spells.areas.get(i).cloned() else {
            continue;
        };
        // On its own cadence, so a late update doesn't lose its last.
        map.spells.areas[i].next_tick = area.next_tick + area.period;
        let Some(s) = data.spells.get(area.spell) else {
            continue;
        };
        if map.unit_view(area.caster).is_none() {
            continue;
        }
        let helpful = auras::positive(s);
        let inside: Vec<Guid> = map
            .grid
            .around(area.at.x, area.at.y, area.radius)
            .filter(|g| guid::is_player(*g) || guid::is_unit(*g))
            .filter(|g| {
                map.position_of(*g)
                    .is_some_and(|p| dist(p, area.at) <= area.radius)
            })
            .filter(|g| map.unit_view(*g).is_some_and(|u| u.alive))
            .filter(|g| {
                if helpful {
                    !combat::can_attack(map, area.caster, *g)
                } else {
                    *g != area.caster && combat::can_attack(map, area.caster, *g)
                }
            })
            .collect();
        let e = s.effects[area.index];
        // One caster's overlapping areas of a spell tick a unit once a
        // period between them (Thistle Tea's `AreaSources`: one holder,
        // shared).
        let gap = area.period.saturating_sub(Duration::from_millis(100));
        let inside: Vec<Guid> = inside
            .into_iter()
            .filter(|g| {
                let key = (area.caster, area.spell, area.index, *g);
                let fresh = map
                    .spells
                    .area_ticked
                    .get(&key)
                    .is_none_or(|t| now.saturating_duration_since(*t) >= gap);
                if fresh {
                    map.spells.area_ticked.insert(key, now);
                }
                fresh
            })
            .collect();
        for g in inside {
            match e.aura {
                AURA_PERIODIC_DAMAGE | AURA_PERIODIC_LEECH => {
                    // Each tick of a ground area rolls its hit on whom it
                    // finds, a resisted one dealing nothing and the area
                    // going on (mangos's `PeriodicTick` for a persistent
                    // area, Thistle Tea's ground ticks); an aura's DoT
                    // doesn't.
                    let hit = outcome::roll_hit(map, area.caster, g, s, &Mods::default());
                    if !hit.lands() {
                        outcome::send_miss(map, area.caster, g, area.spell, hit);
                        continue;
                    }
                    let h = area_holder(&area, s);
                    periodic_damage(
                        map,
                        &h,
                        g,
                        area.amount.max(0) as u32,
                        e.aura == AURA_PERIODIC_LEECH,
                    );
                    if guid::is_unit(g) {
                        crate::game::map::ai::attacked(map, g, area.caster);
                    }
                }
                AURA_PERIODIC_HEAL => {
                    let h = outcome::spell_healing_taken(
                        map,
                        g,
                        area.amount as f32,
                        s,
                        area.index,
                        Kind::Dot,
                        1,
                        1.0,
                    );
                    heal(map, area.caster, g, area.spell, h as u32, true);
                }
                AURA_PERIODIC_TRIGGER_SPELL if e.trigger_spell != 0 => {
                    cast_triggered(map, area.caster, e.trigger_spell, Some(g));
                }
                _ => {
                    // A slow, a debuff: on those inside while they stay.
                    let level = map.unit_view(area.caster).map(|u| u.level).unwrap_or(1);
                    let amount = points(s, area.index, level);
                    let left = area.ends.saturating_duration_since(now).as_millis() as u32;
                    let _ = auras::apply_spell(
                        map,
                        area.caster,
                        g,
                        s,
                        &[(area.index, amount)],
                        auras::Context {
                            duration: Some(Some(left.min(area.period.as_millis() as u32 + 500))),
                            ..Default::default()
                        },
                    );
                }
            }
        }
    }
    let ended: Vec<Guid> = map
        .spells
        .areas
        .iter()
        .filter(|a| a.ends <= now)
        .map(|a| a.object)
        .collect();
    for g in ended {
        super::dynamic::remove(map, g);
    }
    map.spells.areas.retain(|a| a.ends > now);
}

/// Whether a ground area's aura ticks (damage, healing, a trigger).
fn periodic_area(aura: u32) -> bool {
    matches!(
        aura,
        AURA_PERIODIC_DAMAGE
            | AURA_PERIODIC_LEECH
            | AURA_PERIODIC_HEAL
            | AURA_PERIODIC_TRIGGER_SPELL
    )
}

fn area_holder(area: &Area, s: &SpellTemplate) -> auras::Holder {
    auras::Holder {
        spell: area.spell,
        caster: area.caster,
        item: None,
        equipped: false,
        slot: None,
        positive: false,
        passive: false,
        stacks: 1,
        charges: 0,
        expires: None,
        duration: None,
        level: 1,
        school: s.school_mask(),
        mechanic: s.mechanic,
        dispel: s.dispel,
        interrupt_flags: 0,
        item_class: -1,
        item_subclass_mask: 0,
        item_inventory_mask: 0,
        exclusive: None,
        combo: 0,
        proc_ready_at: None,
        effects: Vec::new(),
        heartbeat: None,
        triggered: false,
    }
}

/// Party auras (a paladin's aura, a totem's) follow their caster: every
/// second, party members in range get them, and those out of range lose
/// them.
pub fn refresh_area_auras(map: &mut Map) {
    let now = map.now;
    if map.spells.area_auras_at.is_some_and(|t| t > now) {
        return;
    }
    map.spells.area_auras_at = Some(now + Duration::from_secs(1));
    let data = map.realm.data.clone();
    // The casters' own holders of party auras: players', and those a totem
    // or pet holds of its own (Grounding Totem's magnet), for its master's
    // party.
    type Source = (Guid, u32, Vec<(usize, i32)>, Option<std::time::Instant>);
    let owned: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.alive() && !c.auras.is_empty())
        .map(|(_, c)| c.guid)
        .filter(|g| crate::game::map::pets::owner_of(map, *g).is_some())
        .collect();
    let holders_of = |g: Guid| -> Vec<auras::Holder> {
        auras::auras(map, g)
            .map(|a| a.holders.clone())
            .unwrap_or_default()
    };
    let units: Vec<(Guid, Vec<auras::Holder>)> = map
        .players
        .keys()
        .copied()
        .chain(owned)
        .map(|g| (g, holders_of(g)))
        .collect();
    let sources: Vec<Source> = units
        .iter()
        .flat_map(|(g, holders)| {
            let g = *g;
            holders
                .iter()
                .filter(move |h| h.caster == g)
                .filter(|h| {
                    data.spells.get(h.spell).is_some_and(|s| {
                        s.effects
                            .iter()
                            .any(|e| e.effect == EFFECT_APPLY_AREA_AURA_PARTY)
                    })
                })
                .map(move |h| {
                    (
                        g,
                        h.spell,
                        h.effects
                            .iter()
                            .map(|x| (x.effect as usize, x.amount))
                            .collect(),
                        h.expires,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for (caster, spell, effects, expires) in sources {
        let Some(s) = data.spells.get(spell) else {
            continue;
        };
        let radius = s
            .effects
            .iter()
            .enumerate()
            .filter(|(_, e)| e.effect == EFFECT_APPLY_AREA_AURA_PARTY)
            .map(|(i, _)| data.spells.radius(s.effects[i].radius_index))
            .fold(0.0f32, f32::max)
            .max(30.0);
        let Some(at) = map.position_of(caster) else {
            continue;
        };
        // A totem's or pet's party is its master's, the master too.
        let master = crate::game::map::pets::owner_of(map, caster);
        let leader = master.unwrap_or(caster);
        let party = {
            let groups = map.realm.social.groups.lock();
            groups
                .group_of(leader)
                .map(|g| {
                    let sub = g
                        .members
                        .iter()
                        .find(|m| m.guid == leader)
                        .map(|m| m.subgroup);
                    g.members
                        .iter()
                        .filter(|m| !g.raid || Some(m.subgroup) == sub)
                        .map(|m| m.guid)
                        .filter(|m| *m != caster)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| master.into_iter().collect())
        };
        for member in party {
            let near = map
                .position_of(member)
                .is_some_and(|p| dist(p, at) <= radius)
                && map.unit_view(member).is_some_and(|u| u.alive);
            let has = auras::auras(map, member).is_some_and(|a| {
                a.holders
                    .iter()
                    .any(|h| h.spell == spell && h.caster == caster)
            });
            if near && !has {
                let left = expires.map(|t| t.saturating_duration_since(now).as_millis() as u32);
                let effects: Vec<(usize, i32)> = effects
                    .iter()
                    .copied()
                    .filter(|(i, _)| s.effects[*i].effect == EFFECT_APPLY_AREA_AURA_PARTY)
                    .collect();
                let _ = auras::apply_spell(
                    map,
                    caster,
                    member,
                    s,
                    &effects,
                    auras::Context {
                        duration: Some(left),
                        ..Default::default()
                    },
                );
            } else if !near && has {
                auras::remove_spell(map, member, spell, Some(caster));
            }
        }
    }
    // Party auras whose caster is gone or has dropped them.
    let orphans: Vec<(Guid, u32, Guid)> = map
        .players
        .values()
        .flat_map(|p| {
            p.auras
                .holders
                .iter()
                .filter(|h| h.caster != p.guid)
                .filter(|h| {
                    data.spells.get(h.spell).is_some_and(|s| {
                        s.effects.iter().all(|e| e.effect != EFFECT_APPLY_AURA)
                            && s.effects
                                .iter()
                                .any(|e| e.effect == EFFECT_APPLY_AREA_AURA_PARTY)
                    })
                })
                .map(|h| (p.guid, h.spell, h.caster))
                .collect::<Vec<_>>()
        })
        .collect();
    for (member, spell, caster) in orphans {
        let source = auras::auras(map, caster).is_some_and(|a| {
            a.holders
                .iter()
                .any(|h| h.spell == spell && h.caster == caster)
        });
        if !source {
            auras::remove_spell(map, member, spell, Some(caster));
        }
    }
}

/// The power a unit shows, as an offset from `UNIT_FIELD_POWER1`: a
/// player's by class and form, a creature's by its field.
pub fn shown_power(map: &Map, g: Guid) -> u16 {
    if let Some(p) = map.players.get(&g) {
        return u16::from(p.power_type());
    }
    map.unit_fields(g)
        .map_or(0, |f| u16::from(f.get_byte(UNIT_FIELD_BYTES_0, 3)))
}
