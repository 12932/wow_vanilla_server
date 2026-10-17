//! The class and racial spells whose data can't say what they do, ported
//! from Thistle Tea's `Spell.Scripts` and `Class.*` modules (which follow
//! vmangos's spell scripts): Execute, Life Tap, Conflagrate, Judgement,
//! Holy Shock, Cold Snap, Preparation, Last Stand, Berserking, Blood Fury,
//! Cannibalize, Bloodrage, healthstones, Eviscerate's attack power, and
//! Ignite.


use wow_world_messages::Guid;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat;

use super::outcome::{self, Landing};
use super::*;

const SPELL_EXECUTE_DAMAGE: u32 = 20647;
const SPELL_LIFE_TAP_ENERGIZE: u32 = 31818;
const SPELL_LAST_STAND_HEALTH: u32 = 12976;
const SPELL_BERSERKING_BUFF: u32 = 26635;
const SPELL_BLOOD_FURY_BUFF: u32 = 23234;
const SPELL_BLOOD_FURY_PENALTY: u32 = 23230;
const SPELL_CANNIBALIZE_CHANNEL: u32 = 20578;
const IMPROVED_LIFE_TAP: [u32; 2] = [18182, 18183];
const IMPROVED_HEALTHSTONE: [u32; 2] = [18692, 18693];

const FAMILY_MAGE: u32 = 3;
const FAMILY_WARLOCK: u32 = 5;
const FAMILY_ROGUE: u32 = 8;
const FAMILY_PALADIN: u32 = 10;

/// Holy Shock: its damage and heal spells.
fn holy_shock(id: u32) -> Option<(u32, u32)> {
    match id {
        20473 => Some((25912, 25914)),
        20929 => Some((25911, 25913)),
        20930 => Some((25902, 25903)),
        _ => None,
    }
}

/// The healthstones each Create Healthstone makes, without and with the
/// Improved Healthstone talent's ranks.
fn healthstone(id: u32) -> Option<[u32; 3]> {
    match id {
        6201 => Some([5512, 19004, 19005]),
        6202 => Some([5511, 19006, 19007]),
        5699 => Some([5509, 19008, 19009]),
        11729 => Some([5510, 19010, 19011]),
        11730 => Some([9421, 19012, 19013]),
        _ => None,
    }
}

fn knows(map: &Map, g: Guid, spell: u32) -> bool {
    map.players
        .get(&g)
        .is_some_and(|p| p.spells.contains(&spell))
        || auras::auras(map, g).is_some_and(|a| a.has_spell(spell))
}

fn script(s: &SpellTemplate, name: &str) -> bool {
    s.script_name == name
}

/// Cast `spell` from `caster` at `target` with its effects' points fixed.
pub fn cast_with_points(
    map: &mut Map,
    caster: Guid,
    spell: u32,
    target: Guid,
    points: [Option<i32>; 3],
) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    let mut s = s.clone();
    for (i, p) in points.iter().enumerate() {
        if let Some(p) = p {
            let e = &mut s.effects[i];
            e.base_points = p - 1;
            e.base_dice = 1;
            e.die_sides = 0;
            e.dice_per_level = 0.0;
            e.real_points_per_level = 0.0;
        }
    }
    let targets = Targets {
        unit: Some(target),
        dest: map.position_of(target),
        ..Default::default()
    };
    super::execute(map, caster, &s, targets, None, true);
}

/// A school damage effect a script owns. Returns whether it did it.
pub fn school_damage(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    amount: i32,
    l: &Landing,
) -> bool {
    // Conflagrate needs and spends the caster's Immolate.
    if script(s, "spell_warlock_conflagrate") {
        let immolate = auras::auras(map, target).and_then(|a| {
            a.holders.iter().find(|h| {
                h.caster == caster
                    && map.realm.data.spells.get(h.spell).is_some_and(|t| {
                        t.family_name == FAMILY_WARLOCK && t.family_flags & 0x4 != 0
                    })
            })
        });
        let Some(spell) = immolate.map(|h| h.spell) else {
            return true;
        };
        let d = outcome::damage(map, caster, target, s, index, amount.max(0) as f32, l);
        outcome::deal(map, caster, target, s, d, l);
        auras::remove_spell(map, target, spell, Some(caster));
        return true;
    }
    // Eviscerate: 3% of attack power a combo point more.
    if script(s, "spell_rogue_eviscerate") {
        let ap = map
            .players
            .get(&caster)
            .map_or(0, |p| p.derived.attack_power) as f32;
        let extra = ap * f32::from(l.combo) * 0.03;
        let d = outcome::damage(
            map,
            caster,
            target,
            s,
            index,
            amount.max(0) as f32 + extra,
            l,
        );
        outcome::deal(map, caster, target, s, d, l);
        return true;
    }
    false
}

/// A dummy or script effect.
#[allow(clippy::too_many_arguments)]
pub fn dummy(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    amount: i32,
    l: &Landing,
    item: Option<Guid>,
) {
    let data = map.realm.data.clone();
    let _ = item;
    // Holy Light and Flash of Light: the heal, with Blessing of Light's
    // bonus on the target.
    if s.family_name == FAMILY_PALADIN
        && s.family_flags & 0xC000_0000 != 0
        && s.effects[index].effect == EFFECT_SCRIPT_EFFECT
    {
        let holy_light = s.family_flags & 0x8000_0000 != 0;
        let bonus = blessing_of_light(map, target, if holy_light { 0 } else { 1 });
        let heal = if holy_light { 19968 } else { 19993 };
        cast_with_points(
            map,
            caster,
            heal,
            target,
            [Some(amount + bonus), None, None],
        );
        return;
    }
    match s.script_name.as_str() {
        "spell_warrior_execute_dummy" => {
            // The rage left is damage.
            let rage = map
                .unit_fields(caster)
                .map(|f| f.get_u32(UNIT_FIELD_POWER1 + 1))
                .unwrap_or(0);
            let damage = amount + (rage as f32 * s.dmg_multiplier[index]) as i32;
            if let Some(f) = map.unit_fields_mut(caster) {
                f.set_u32(UNIT_FIELD_POWER1 + 1, 0);
            }
            let mut strike = data
                .spells
                .get(SPELL_EXECUTE_DAMAGE)
                .cloned()
                .unwrap_or_else(|| s.clone());
            strike.dmg_class = 2;
            let d = outcome::damage(map, caster, target, &strike, 0, damage.max(0) as f32, l);
            outcome::deal(map, caster, target, s, d, l);
            return;
        }
        "spell_warlock_life_tap" => {
            let bonus = outcome::bonus(
                map,
                caster,
                caster,
                s,
                index,
                outcome::Kind::Direct,
                &l.mods,
            );
            let cost = l.mods.apply(mods::OP_COST, amount as f32) + bonus;
            let health = map
                .unit_fields(caster)
                .map(|f| f.get_u32(UNIT_FIELD_HEALTH))
                .unwrap_or(0);
            if (health as f32) <= cost {
                return;
            }
            if let Some(f) = map.unit_fields_mut(caster) {
                f.set_u32(UNIT_FIELD_HEALTH, health - cost as u32);
            }
            let improved = IMPROVED_LIFE_TAP
                .iter()
                .position(|id| knows(map, caster, *id))
                .map_or(0, |i| (i as i32 + 1) * 10);
            let gain = cost as i32 * (100 + improved) / 100;
            cast_with_points(
                map,
                caster,
                SPELL_LIFE_TAP_ENERGIZE,
                caster,
                [Some(gain), None, None],
            );
            return;
        }
        "spell_mage_cold_snap" => {
            reset_cooldowns(map, caster, s.id, |t| {
                t.family_name == FAMILY_MAGE && t.school_mask() & 0x10 != 0
            });
            return;
        }
        "spell_paladin_holy_shock" => {
            if let Some((damage, heal)) = holy_shock(s.id) {
                let spell = if combat::can_attack(map, caster, target) {
                    damage
                } else {
                    heal
                };
                cast_triggered(map, caster, spell, Some(target));
            }
            return;
        }
        "spell_cannibalize" => {
            let corpse = map.creatures.iter().any(|(_, c)| {
                !c.alive()
                    && map
                        .realm
                        .data
                        .creature_template(c.entry)
                        .is_some_and(|t| matches!(t.creature_type, 6 | 7))
                    && map
                        .position_of(caster)
                        .is_some_and(|p| combat::dist(p, c.position) <= 5.0)
            });
            if corpse {
                cast_triggered(map, caster, SPELL_CANNIBALIZE_CHANNEL, Some(caster));
            }
            return;
        }
        "spell_warlock_create_healthstone" => {
            if let Some(items) = healthstone(s.id) {
                let rank = IMPROVED_HEALTHSTONE
                    .iter()
                    .rposition(|id| knows(map, caster, *id))
                    .map_or(0, |i| i + 1);
                let _ = crate::game::map::inventory::add_item(map, caster, items[rank], 1, true);
            }
            return;
        }
        _ => {}
    }
    match s.id {
        // Last Stand: three tenths of maximum health, for a while.
        12975 => {
            let max = map
                .unit_fields(caster)
                .map(|f| f.get_u32(UNIT_FIELD_MAXHEALTH))
                .unwrap_or(0);
            let amount = (max as f32 * 0.3) as i32;
            cast_with_points(
                map,
                caster,
                SPELL_LAST_STAND_HEALTH,
                caster,
                [Some(amount), None, None],
            );
        }
        // Preparation: the rogue's cooldowns are done.
        14185 => reset_cooldowns(map, caster, s.id, |t| t.family_name == FAMILY_ROGUE),
        // Blood Fury: a share of attack power, and the healing penalty.
        20572 if index == 0 || s.effects[0].effect != EFFECT_DUMMY => {
            let ap = map
                .players
                .get(&caster)
                .map_or(0, |p| p.derived.attack_power) as f32;
            let bonus = (ap * amount.max(0) as f32 / 100.0) as i32;
            cast_triggered(map, caster, SPELL_BLOOD_FURY_PENALTY, Some(caster));
            if bonus > 0 {
                cast_with_points(
                    map,
                    caster,
                    SPELL_BLOOD_FURY_BUFF,
                    caster,
                    [Some(bonus), None, None],
                );
            }
        }
        _ if s.family_name == 0 && s.spell_icon == 1661 => {
            // Berserking: more haste the more hurt.
            let pct = map
                .unit_fields(caster)
                .map(|f| {
                    let max = f.get_u32(UNIT_FIELD_MAXHEALTH).max(1);
                    let h = f.get_u32(UNIT_FIELD_HEALTH) * 100 / max;
                    10 + (100 - h.clamp(40, 100)) as i32 / 3
                })
                .unwrap_or(10);
            cast_with_points(
                map,
                caster,
                SPELL_BERSERKING_BUFF,
                caster,
                [Some(pct), Some(pct), Some(pct)],
            );
        }
        _ => {
            if s.effects[index].effect == EFFECT_SCRIPT_EFFECT && judgement(map, caster, target, s)
            {
                return;
            }
            // A dummy that names a spell to trigger.
            let e = s.effects[index];
            if e.trigger_spell != 0 && data.spells.get(e.trigger_spell).is_some() {
                cast_triggered(map, caster, e.trigger_spell, Some(target));
            }
        }
    }
}

/// Blessing of Light's bonus to Holy Light (effect 0) or Flash of Light
/// (effect 1) on `target`.
fn blessing_of_light(map: &Map, target: Guid, index: u8) -> i32 {
    let data = &map.realm.data;
    auras::auras(map, target).map_or(0, |a| {
        a.effects()
            .filter(|(h, x)| {
                x.kind == AURA_DUMMY
                    && x.effect == index
                    && data.spells.get(h.spell).is_some_and(|t| {
                        t.family_name == FAMILY_PALADIN
                            && t.family_flags & 0x1000_0000 != 0
                            && t.spell_visual == 300
                    })
            })
            .map(|(h, x)| x.amount * h.stacks.max(1) as i32)
            .sum()
    })
}

/// Judgement: the paladin's seal is spent for its judgement at the target.
fn judgement(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate) -> bool {
    if s.family_name != FAMILY_PALADIN || s.family_flags & 0x80_0000 == 0 {
        return false;
    }
    let seal = auras::auras(map, caster).and_then(|a| {
        a.holders
            .iter()
            .find(|h| h.exclusive == Some(auras::Exclusive::PaladinSeal))
            .map(|h| {
                (
                    h.spell,
                    h.effects
                        .iter()
                        .find(|x| x.effect == 2 && x.kind == AURA_DUMMY)
                        .map(|x| x.amount),
                )
            })
    });
    let Some((seal, judgement)) = seal else {
        return true;
    };
    auras::remove_spell(map, caster, seal, Some(caster));
    if let Some(spell) = judgement.filter(|j| *j > 1) {
        cast_triggered(map, caster, spell as u32, Some(target));
    }
    true
}

/// The cooldowns of `caster`'s spells that `which` picks end now, but
/// `except`'s.
pub fn reset_cooldowns(
    map: &mut Map,
    caster: Guid,
    except: u32,
    which: impl Fn(&SpellTemplate) -> bool,
) {
    let data = map.realm.data.clone();
    let spells: Vec<u32> = map
        .caster(caster)
        .map(|c| {
            c.cooldowns
                .keys()
                .copied()
                .filter(|id| *id != except && data.spells.get(*id).is_some_and(&which))
                .collect()
        })
        .unwrap_or_default();
    for id in spells {
        let category = data.spells.get(id).map_or(0, |t| t.category);
        if let Some(c) = map.caster_mut(caster) {
            c.cooldowns.remove(&id);
            if category != 0 {
                c.category_cooldowns.remove(&category);
            }
        }
        super::cooldowns::send_clear(map, caster, id);
    }
}

/// A druid changed form: Furor's chance of rage or energy, and Leader of
/// the Pack's aura in cat and bear form.
pub fn shapeshifted(map: &mut Map, g: Guid, before: u8, form: u8) {
    use crate::game::map::auras::{FORM_BEAR, FORM_CAT, FORM_DIREBEAR};
    let feral = |f: u8| matches!(f, FORM_CAT | FORM_BEAR | FORM_DIREBEAR);
    if feral(form) {
        let furor = auras::auras(map, g).map_or(0, |a| {
            a.effects()
                .filter(|(h, x)| {
                    matches!(h.spell, 17056 | 17058 | 17059 | 17060 | 17061) && x.kind == AURA_DUMMY
                })
                .map(|(_, x)| x.amount)
                .max()
                .unwrap_or(0)
        });
        if furor > 0 && outcome::chance(map, furor as f32) {
            let (power, gain) = if form == FORM_CAT {
                (3u16, 40u32)
            } else {
                (1, 100)
            };
            if let Some(f) = map.unit_fields_mut(g) {
                let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
                let now = f.get_u32(UNIT_FIELD_POWER1 + power);
                f.set_u32(UNIT_FIELD_POWER1 + power, (now + gain).min(max));
            }
        }
        if knows(map, g, 17007) && !auras::auras(map, g).is_some_and(|a| a.has_spell(24932)) {
            cast_triggered(map, g, 24932, Some(g));
        }
    } else if feral(before) {
        auras::remove_spell(map, g, 24932, Some(g));
    }
}

/// After an energize: Bloodrage keeps the warrior in combat.
pub fn after_energize(map: &mut Map, caster: Guid, s: &SpellTemplate) {
    if script(s, "spell_warrior_bloodrage")
        && let Some(p) = map.players.get_mut(&caster)
    {
        p.fields.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            true,
        );
        p.combat.last_action = Some(map.now);
    }
}

/// The item a create-item effect makes: its own, or a script's choice.
pub fn created_item(map: &Map, caster: Guid, s: &SpellTemplate, item: u32) -> u32 {
    if let Some(items) = healthstone(s.id) {
        let rank = IMPROVED_HEALTHSTONE
            .iter()
            .rposition(|id| knows(map, caster, *id))
            .map_or(0, |i| i + 1);
        return items[rank];
    }
    item
}
