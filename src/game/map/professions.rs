//! Professions: gathering herbs and ore, picking locks and pockets,
//! skinning, fishing, disenchanting and enchanting, and the skill each
//! raises; crafting's checks (a spell focus near, the tools carried) and
//! its skill-ups by a recipe's colour. The rates are Thistle Tea's
//! (`Core.Profession.*`), which are vmangos's defaults.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    MSG_CHANNEL_UPDATE_Server, SMSG_FISH_ESCAPED, SMSG_FISH_NOT_HOOKED,
    SMSG_GAMEOBJECT_CUSTOM_ANIM, Vector3d,
};

use crate::game::data::crafts::{LOCK_KEY_ITEM, LOCK_KEY_SKILL};
use crate::game::data::spells::SpellTemplate;
use crate::game::data::{GameData, GameObjectSpawn};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_MOD_SKILL, AURA_MOD_SKILL_TALENT, AURA_MOD_STEALTH};
use crate::game::map::combat::dist;
use crate::game::map::gameobject::GameObject;
use crate::game::map::loot::{self, Loot, LootKind};
use crate::game::map::player::Player;
use crate::game::map::skills;
use crate::game::map::spells::Targets;

pub const SKILL_FIRST_AID: u32 = 129;
pub const SKILL_BLACKSMITHING: u32 = 164;
pub const SKILL_LEATHERWORKING: u32 = 165;
pub const SKILL_ALCHEMY: u32 = 171;
pub const SKILL_HERBALISM: u32 = 182;
pub const SKILL_COOKING: u32 = 185;
pub const SKILL_MINING: u32 = 186;
pub const SKILL_TAILORING: u32 = 197;
pub const SKILL_ENGINEERING: u32 = 202;
pub const SKILL_ENCHANTING: u32 = 333;
pub const SKILL_FISHING: u32 = 356;
pub const SKILL_SKINNING: u32 = 393;
pub const SKILL_LOCKPICKING: u32 = 633;

/// The skills recipes raise: smelting raises Mining.
const CRAFTS: &[u32] = &[
    SKILL_FIRST_AID,
    SKILL_BLACKSMITHING,
    SKILL_LEATHERWORKING,
    SKILL_ALCHEMY,
    SKILL_COOKING,
    SKILL_MINING,
    SKILL_TAILORING,
    SKILL_ENGINEERING,
    SKILL_ENCHANTING,
];

// Effects.
pub const EFFECT_TRANS_DOOR: u32 = 50;
pub const EFFECT_ENCHANT_ITEM: u32 = 53;
pub const EFFECT_ENCHANT_ITEM_TEMPORARY: u32 = 54;
pub const EFFECT_OPEN_LOCK: u32 = 33;
pub const EFFECT_OPEN_LOCK_ITEM: u32 = 59;
pub const EFFECT_PICKPOCKET: u32 = 71;
pub const EFFECT_SKINNING: u32 = 95;
/// A bite's splash.
const BITE_SOUND: u32 = 3355;
/// Skinning's knives: any of them serves (Thistle Tea's `check_tool`).
pub const SKINNING_TOOLS: [u32; 3] = [7005, 12_709, 19_901];
/// The Beast of Blackrock Spire, and the spell skinning it frees Finkle
/// Einhorn with (vmangos' `boss_the_beast`).
pub const THE_BEAST: u32 = 10_430;
pub const FINKLE_IS_EINHORN: u32 = 16_710;

/// Why a spell's tool is missing: a skinning knife may be any of the
/// knives that skin.
pub fn tool_missing(p: &Player, s: &SpellTemplate, tool: u32) -> Option<u8> {
    use crate::game::map::spells::checks::fail;
    if p.carried_count_of(tool) > 0 {
        return None;
    }
    if SKINNING_TOOLS.contains(&tool) && s.has_effect(EFFECT_SKINNING) {
        return (!SKINNING_TOOLS.iter().any(|t| p.carried_count_of(*t) > 0))
            .then_some(fail::EQUIPPED_ITEM);
    }
    Some(fail::TOTEMS)
}
pub const EFFECT_DISENCHANT: u32 = 99;

// Game object types.
pub const GO_DOOR: u8 = 0;
pub const GO_BUTTON: u8 = 1;
pub const GO_CHEST: u8 = 3;
pub const GO_SPELL_FOCUS: u8 = 8;
pub const GO_GOOBER: u8 = 10;
pub const GO_FISHING_NODE: u8 = 17;
pub const GO_FISHING_HOLE: u8 = 25;

/// `UNIT_FIELD_FLAGS`: the corpse can be skinned.
pub const UNIT_FLAG_SKINNABLE: u32 = 0x0400_0000;
/// `ITEM_FIELD_FLAGS`: a lockbox picked open.
pub const ITEM_FLAG_UNLOCKED: u32 = 0x4;
/// Item flags: it can't be disenchanted.
const ITEM_FLAG_NO_DISENCHANT: u32 = 0x8000;

/// The fishing bobber, and its catch.
pub const BOBBER: u32 = 35591;
const BITE_DELAYS_MS: [u64; 4] = [13_000, 17_000, 23_000, 27_000];
const CATCH_WINDOW: Duration = Duration::from_secs(5);
const FISHING_HOLE_RANGE: f32 = 22.0;
/// How near an object a lock spell is cast from (Thistle Tea's
/// `ObjectTarget`, the interaction range).
const LOCK_YARDS: f32 = 5.0;

// `SpellCastResult`s.
const FAIL_ALREADY_OPEN: u8 = 0x06;
const FAIL_BAD_TARGETS: u8 = 0x0a;
const FAIL_CANT_BE_DISENCHANTED: u8 = 0x0c;
const FAIL_CHEST_IN_USE: u8 = 0x15;
const FAIL_ITEM_GONE: u8 = 0x26;
const FAIL_LOW_CASTLEVEL: u8 = 0x2c;
const FAIL_NOT_FISHABLE: u8 = 0x34;
const FAIL_ONLY_STEALTHED: u8 = 0x57;
const FAIL_OUT_OF_RANGE: u8 = 0x59;
pub const FAIL_REQUIRES_SPELL_FOCUS: u8 = 0x5e;
const FAIL_TARGETS_DEAD: u8 = 0x65;
const FAIL_TARGET_NOT_DEAD: u8 = 0x6e;
const FAIL_TARGET_NOT_LOOTED: u8 = 0x70;
const FAIL_TARGET_NO_POCKETS: u8 = 0x72;
const FAIL_TARGET_UNSKINNABLE: u8 = 0x74;
const FAIL_TRY_AGAIN: u8 = 0x7a;

/// A bobber in the water.
#[derive(Debug, Clone)]
pub struct Bobber {
    pub guid: Guid,
    pub owner: Guid,
    pub spell: u32,
    pub bite_at: Instant,
    pub gone_at: Instant,
    pub bitten: bool,
    pub zone: u32,
    pub area: u32,
}

/// A lock type's skill.
fn lock_skill(lock_type: u32) -> Option<u32> {
    match lock_type {
        1 => Some(SKILL_LOCKPICKING),
        2 => Some(SKILL_HERBALISM),
        3 => Some(SKILL_MINING),
        19 => Some(SKILL_FISHING),
        _ => None,
    }
}

/// A skill's value with its aura bonuses, or 0 unknown.
pub fn skill_value(p: &Player, skill: u32) -> u32 {
    let base = skills::value(p, skill).unwrap_or(0);
    if base == 0 {
        return 0;
    }
    let bonus = p.auras.sum_misc(AURA_MOD_SKILL, skill as i32)
        + p.auras.sum_misc(AURA_MOD_SKILL_TALENT, skill as i32);
    (base as i32 + bonus).max(0) as u32
}

/// A game object's lock, 0 for none.
pub fn object_lock(data: &GameData, entry: u32) -> u32 {
    data.gameobject_templates
        .get(&entry)
        .map(|t| match t.kind {
            GO_DOOR | GO_BUTTON => t.data[1],
            // A trap's lock is what Disarm Trap opens (mangos's
            // `GameObjectInfo::GetLockId`).
            GO_CHEST | GO_GOOBER | crate::game::map::objects::GO_TRAP => t.data[0],
            _ => 0,
        })
        .unwrap_or(0)
}

/// What opening a lock comes to: the skill it took, with its value and
/// the value needed, and whether the attempt may raise it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Opening {
    pub skill: Option<u32>,
    pub value: u32,
    pub required: u32,
    pub gain: bool,
}

/// Blasting's lock type (the seaforium charges): no skill, but the
/// charge's strength against the lock's need.
const LOCK_BLASTING: u32 = 16;

/// How `s`'s effect `index` opens `lock`, cast from an item of
/// `cast_item` or by hand, as Thistle Tea's `OpenLock.resolve`: the lock's
/// ways in order, the first that fits deciding. An item way is the item
/// cast; a skill way the player's skill and the spell's points over one
/// (an item cast has only its own), short of the need too low.
pub fn resolve_lock(
    data: &GameData,
    p: &Player,
    s: &SpellTemplate,
    index: usize,
    lock: u32,
    cast_item: Option<u32>,
) -> Result<Opening, u8> {
    let Some(keys) = data.crafts.locks.get(&lock) else {
        return Err(FAIL_BAD_TARGETS);
    };
    let e = &s.effects[index];
    let lock_type = e.misc_value.max(0) as u32;
    let bonus = (e.base_points + 1).max(0) as u32;
    for k in keys {
        match k.kind {
            LOCK_KEY_ITEM if k.index != 0 && cast_item == Some(k.index) => {
                return Ok(Opening::default());
            }
            LOCK_KEY_SKILL if k.index == lock_type => {
                let skill = lock_skill(lock_type);
                let known = skill.is_some_and(|sk| skills::value(p, sk).is_some_and(|v| v > 0));
                let own = match (cast_item, skill) {
                    (None, Some(sk)) => skill_value(p, sk),
                    _ => 0,
                };
                let value = own + bonus;
                let required = if skill.is_some() || lock_type == LOCK_BLASTING {
                    k.skill
                } else {
                    0
                };
                if value < required {
                    return Err(FAIL_LOW_CASTLEVEL);
                }
                return Ok(Opening {
                    skill,
                    value,
                    required,
                    gain: cast_item.is_none() && known,
                });
            }
            _ => {}
        }
    }
    Err(FAIL_BAD_TARGETS)
}

/// Whether an attempt at a lock of gathering or lockpicking takes: the
/// skill rolled from 25 below to 37 above must reach the need, but herbs
/// and ore can't be missed from 300 (Thistle Tea's `Gathering.attempt?`).
fn attempt_takes(o: &Opening) -> bool {
    let Some(skill) = o
        .skill
        .filter(|sk| matches!(*sk, SKILL_HERBALISM | SKILL_MINING | SKILL_LOCKPICKING))
    else {
        return true;
    };
    let roll = o.value as i32 - 26 + rand::rng().random_range(1..=63);
    (matches!(skill, SKILL_HERBALISM | SKILL_MINING) && o.value >= 300) || o.required as i32 <= roll
}

/// A lock's skill raised, by the learned skill against the need.
fn lock_skill_up(map: &mut Map, caster: Guid, o: &Opening) -> bool {
    let Some(skill) = o.skill.filter(|_| o.gain) else {
        return false;
    };
    let learned = map
        .players
        .get(&caster)
        .and_then(|p| skills::value(p, skill))
        .unwrap_or(0);
    try_raise(
        map,
        caster,
        skill,
        gather_gain_chance(skill, learned, o.required, 1),
    )
}

/// The item a cast was from, by its entry, among `p`'s.
fn entry_of(p: &Player, item: Option<Guid>) -> Option<u32> {
    let g = item?;
    p.items.iter().find(|i| i.guid == g).map(|i| i.entry)
}

/// Why a lock spell can't be cast at the lockbox `item` (Thistle Tea's
/// `target_lock` and `ItemOpening.validate_unlock`): gone, already open,
/// or the lock too much.
fn lockbox_fails(
    data: &GameData,
    p: &Player,
    s: &SpellTemplate,
    index: usize,
    item: Guid,
    cast_item: Option<u32>,
) -> Option<u8> {
    let Some(item) = p.items.iter().find(|i| i.guid == item && i.carried()) else {
        return Some(FAIL_ITEM_GONE);
    };
    let lock = data.item(item.entry).map_or(0, |t| t.lock_id);
    if lock == 0 || item.fields.get_u32(ITEM_FIELD_FLAGS) & ITEM_FLAG_UNLOCKED != 0 {
        return Some(FAIL_ALREADY_OPEN);
    }
    resolve_lock(data, p, s, index, lock, cast_item).err()
}

/// The skill a creature's skin needs.
pub fn skinning_required(level: u32, skill: u32) -> u32 {
    if skill < 100 {
        level.saturating_sub(10) * 10
    } else {
        level * 5
    }
}

/// A gathering skill's chance to rise, in percent, not rounded (Thistle
/// Tea's `Gathering.gain_chance`).
pub fn gather_gain_chance(skill: u32, value: u32, required: u32, multiplier: u32) -> f64 {
    let chance = if value >= required + 100 {
        0.0
    } else if value >= required + 50 {
        25.0
    } else if value >= required + 25 {
        75.0
    } else {
        100.0
    };
    let divisor = if matches!(skill, SKILL_MINING | SKILL_SKINNING) {
        f64::from(1u32 << (value / 75).min(8))
    } else {
        1.0
    };
    (chance * f64::from(multiplier) / divisor).min(100.0)
}

/// A recipe's chance to raise its skill: orange, yellow, green, grey.
pub fn craft_gain_chance(value: u32, yellow: u32, gray: u32) -> u32 {
    if value >= gray {
        0
    } else if value >= (yellow + gray) / 2 {
        25
    } else if value >= yellow {
        75
    } else {
        100
    }
}

/// Raise `skill` by one, `chance` percent of the time (a roll in
/// `[0, 100)` under it, as Thistle Tea rolls).
pub fn try_raise(map: &mut Map, player: Guid, skill: u32, chance: f64) -> bool {
    let roll = rand::rng().random_range(0.0..100.0);
    roll < chance
        && map
            .players
            .get_mut(&player)
            .is_some_and(|p| skills::raise(p, skill, 1))
}

fn send_failure(map: &Map, player: Guid, spell: u32, code: u8) {
    if let Some(p) = map.players.get(&player) {
        let mut b = crate::game::data::items::Body::default();
        b.u32(spell).u8(2).u8(code);
        p.outbound
            .send_frame(b.frame(crate::game::map::spells::SMSG_CAST_RESULT));
    }
}

/// Whether this module runs effect `index` of `s`.
pub fn handles(data: &GameData, s: &SpellTemplate, index: usize) -> bool {
    let e = &s.effects[index];
    match e.effect {
        EFFECT_OPEN_LOCK
        | EFFECT_OPEN_LOCK_ITEM
        | EFFECT_PICKPOCKET
        | EFFECT_SKINNING
        | EFFECT_DISENCHANT
        | EFFECT_ENCHANT_ITEM
        | EFFECT_ENCHANT_ITEM_TEMPORARY => true,
        EFFECT_TRANS_DOOR => data
            .gameobject_templates
            .get(&(e.misc_value.max(0) as u32))
            .is_some_and(|t| t.kind == GO_FISHING_NODE),
        _ => false,
    }
}

/// Why `caster` can't cast `s` at `t` (from `cast_item`, if an item cast
/// it) for its profession's sake: a spell focus not near, tools not
/// carried, a lock beyond the skill, a target that can't be skinned,
/// picked or disenchanted.
pub fn check(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    cast_item: Option<Guid>,
) -> Option<u8> {
    let data = &map.realm.data;
    let p = map.players.get(&caster)?;
    let cast_item = entry_of(p, cast_item);
    // Skinning's corpse before its knife (Thistle Tea's `Skinning.validate`).
    if s.has_effect(EFFECT_SKINNING)
        && let Some(code) = skinning_fails(map, p, t)
    {
        return Some(code);
    }
    if s.requires_spell_focus != 0 && !focus_near(map, caster, s.requires_spell_focus) {
        return Some(FAIL_REQUIRES_SPELL_FOCUS);
    }
    if let Some(code) = s
        .totems
        .iter()
        .filter(|t| **t != 0)
        .find_map(|t| tool_missing(p, s, *t))
    {
        return Some(code);
    }
    for (index, e) in s.effects.iter().enumerate() {
        match e.effect {
            // A chest, node or door (or a rune a Quintessence douses): near,
            // not in use, locked, and the lock not too much. The object
            // first, as vmangos's `EffectOpenLock` has it: a key cast at a
            // door is the target item too.
            EFFECT_OPEN_LOCK | EFFECT_OPEN_LOCK_ITEM if t.object.is_some() => {
                let o = t.object.and_then(|g| map.gameobject(g))?;
                if dist(p.position(), o.position) > LOCK_YARDS {
                    return Some(FAIL_OUT_OF_RANGE);
                }
                if o.loot.as_ref().is_some_and(|l| !l.looters.is_empty()) {
                    return Some(FAIL_CHEST_IN_USE);
                }
                let lock = object_lock(data, o.entry);
                if lock == 0 {
                    return Some(FAIL_ALREADY_OPEN);
                }
                if let Err(code) = resolve_lock(data, p, s, index, lock, cast_item) {
                    return Some(code);
                }
            }
            // A lockbox in the bags: Pick Lock's, or a key's.
            EFFECT_OPEN_LOCK | EFFECT_OPEN_LOCK_ITEM if t.item.is_some() => {
                if let Some(code) = t
                    .item
                    .and_then(|item| lockbox_fails(data, p, s, index, item, cast_item))
                {
                    return Some(code);
                }
            }
            EFFECT_OPEN_LOCK_ITEM => return Some(FAIL_ITEM_GONE),
            EFFECT_PICKPOCKET => {
                if !p.auras.has(AURA_MOD_STEALTH) {
                    return Some(FAIL_ONLY_STEALTHED);
                }
                let c = t.unit.and_then(|u| map.creature(u));
                let Some(c) = c else {
                    return Some(FAIL_BAD_TARGETS);
                };
                if !c.alive() {
                    return Some(FAIL_TARGETS_DEAD);
                }
                let pockets = data
                    .creature_template(c.entry)
                    .is_some_and(|t| t.pickpocket_loot_id != 0);
                if !pockets {
                    return Some(FAIL_TARGET_NO_POCKETS);
                }
            }
            // Thistle Tea's `Disenchant.validate_item`: an enchanter, and a
            // single carried item that may be.
            EFFECT_DISENCHANT => {
                if skills::value(p, SKILL_ENCHANTING).unwrap_or(0) == 0 {
                    return Some(FAIL_LOW_CASTLEVEL);
                }
                let item = t
                    .item
                    .and_then(|g| p.items.iter().find(|i| i.guid == g && i.carried()));
                let ok = item.is_some_and(|i| {
                    i.count() == 1
                        && data.item(i.entry).is_some_and(|t| {
                            t.disenchant_id != 0 && t.flags & ITEM_FLAG_NO_DISENCHANT == 0
                        })
                });
                if !ok {
                    return Some(FAIL_CANT_BE_DISENCHANTED);
                }
            }
            EFFECT_ENCHANT_ITEM | EFFECT_ENCHANT_ITEM_TEMPORARY => {
                let item = t
                    .item
                    .and_then(|g| p.items.iter().find(|i| i.guid == g && i.carried()));
                let Some(item) = item else {
                    return Some(FAIL_ITEM_GONE);
                };
                let fits = data.item(item.entry).is_some_and(|it| {
                    (s.equipped_item_class < 0 || it.class as i32 == s.equipped_item_class)
                        && (s.equipped_item_subclass_mask <= 0
                            || s.equipped_item_subclass_mask & (1 << it.subclass) != 0)
                        && (s.equipped_item_inventory_type_mask <= 0
                            || s.equipped_item_inventory_type_mask & (1 << it.inventory_type) != 0)
                });
                if !fits || item.count() != 1 {
                    return Some(FAIL_BAD_TARGETS);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether a corpse has a skin still on it: its kind has skins, and it
/// isn't skinned.
fn has_skin(data: &GameData, c: &crate::game::map::creature::Creature) -> bool {
    data.creature_template(c.entry)
        .is_some_and(|t| t.skinning_loot_id != 0)
        && !c
            .loot
            .as_ref()
            .is_some_and(|l| l.kind == LootKind::Skinning)
}

/// Why a corpse can't be skinned, in Thistle Tea's order: not a wild
/// creature's, alive, without a skin, still with its loot, beyond the
/// skill.
fn skinning_fails(map: &Map, p: &Player, t: &Targets) -> Option<u8> {
    let data = &map.realm.data;
    let Some(c) = t
        .unit
        .and_then(|u| map.creature(u))
        .filter(|c| c.owner.is_none())
    else {
        return Some(FAIL_BAD_TARGETS);
    };
    if c.alive() {
        return Some(FAIL_TARGET_NOT_DEAD);
    }
    if !has_skin(data, c) {
        return Some(FAIL_TARGET_UNSKINNABLE);
    }
    if c.loot.as_ref().is_some_and(|l| !l.is_empty()) {
        return Some(FAIL_TARGET_NOT_LOOTED);
    }
    let level = c.fields.get_u32(UNIT_FIELD_LEVEL);
    let skill = skill_value(p, SKILL_SKINNING);
    if skill == 0 || skinning_required(level, skill) > skill {
        return Some(FAIL_LOW_CASTLEVEL);
    }
    None
}

/// A spell focus of `focus` within its reach of `caster`: closer than its
/// radius and both bodies' (Thistle Tea's `Focus.range`, mangos's
/// `IsWithinDistInMap`).
pub fn focus_near(map: &Map, caster: Guid, focus: u32) -> bool {
    const OBJECT_RADIUS: f32 = 0.389;
    let data = &map.realm.data;
    let Some((at, own)) = map
        .unit_fields(caster)
        .zip(map.position_of(caster))
        .map(|(f, at)| {
            let r = f.get_f32(UNIT_FIELD_BOUNDINGRADIUS);
            (at, if r > 0.0 { r } else { OBJECT_RADIUS })
        })
    else {
        return false;
    };
    map.grid
        .around(at.x, at.y, 50.0)
        .filter(|g| guid::is_game_object(*g))
        .filter_map(|g| map.gameobject(g))
        .any(|o| {
            data.gameobject_templates.get(&o.entry).is_some_and(|t| {
                t.kind == GO_SPELL_FOCUS
                    && t.data[0] == focus
                    && dist(o.position, at) < t.data[1] as f32 + own + OBJECT_RADIUS
            })
        })
}

/// Run `s`'s profession effects at `t`, cast with an item of entry
/// `cast_item` if any; the effects run are returned as a mask. An
/// enchantment made by hand raises its skill (Thistle Tea's
/// `advance_skill`); one from an item, or for a time, doesn't.
pub fn execute(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    cast_item: Option<u32>,
) -> u8 {
    let data = map.realm.data.clone();
    let mut done = 0u8;
    for index in 0..3 {
        if s.effects[index].effect == 0 || !handles(&data, s, index) {
            continue;
        }
        done |= 1 << index;
        match s.effects[index].effect {
            EFFECT_OPEN_LOCK | EFFECT_OPEN_LOCK_ITEM => match (t.object, t.item) {
                (Some(object), _) => open_lock(map, caster, s, index, object, cast_item),
                (None, Some(item)) => open_lock_item(map, caster, s, index, item, cast_item),
                _ => {}
            },
            EFFECT_SKINNING => {
                if let Some(unit) = t.unit {
                    skin(map, caster, unit, s.id);
                }
            }
            EFFECT_PICKPOCKET => {
                if let Some(unit) = t.unit {
                    pick_pocket(map, caster, unit);
                }
            }
            EFFECT_DISENCHANT => {
                if let Some(item) = t.item {
                    disenchant(map, caster, item, s.id);
                }
            }
            EFFECT_ENCHANT_ITEM | EFFECT_ENCHANT_ITEM_TEMPORARY => {
                if let Some(item) = t.item {
                    let id = s.effects[index].misc_value.max(0) as u32;
                    let temporary = (s.effects[index].effect == EFFECT_ENCHANT_ITEM_TEMPORARY)
                        .then(|| {
                            let secs = crate::game::map::spells::points(s, index, 1).max(1) as u32;
                            let charges =
                                data.crafts.enchant_charges.get(&s.id).copied().unwrap_or(0);
                            (secs * 1000, charges)
                        });
                    let enchanted =
                        crate::game::map::enchant::enchant_item(map, caster, item, id, temporary);
                    if enchanted && temporary.is_none() && cast_item.is_none() {
                        recipe_skill_up(map, caster, s);
                    }
                }
            }
            EFFECT_TRANS_DOOR => cast_line(map, caster, s, index),
            _ => {}
        }
    }
    done
}

/// A recipe made: its skill may rise, by the recipe's colour. The first of
/// its rows a craft names counts (Thistle Tea's `Skill.recipe`).
pub fn recipe_skill_up(map: &mut Map, caster: Guid, s: &SpellTemplate) {
    let data = map.realm.data.clone();
    let Some(a) = data.spells.skill_abilities.get(&s.id).and_then(|rows| {
        rows.iter()
            .find(|a| CRAFTS.contains(&a.skill) && a.trivial_high > 0)
    }) else {
        return;
    };
    let Some(value) = map
        .players
        .get(&caster)
        .and_then(|p| skills::value(p, a.skill))
    else {
        return;
    };
    let chance = craft_gain_chance(value, a.trivial_low, a.trivial_high);
    try_raise(map, caster, a.skill, f64::from(chance));
}

/// Gathering, lockpicking or a plain opening: a lock opened with skill
/// may fail, and may raise the skill; a chest or node then shows its loot,
/// a door opens.
fn open_lock(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    object: Guid,
    cast_item: Option<u32>,
) {
    let data = map.realm.data.clone();
    let Some(entry) = map.gameobject(object).map(|o| o.entry) else {
        return;
    };
    // Another's window opened on it in the meantime (Thistle Tea's check
    // at completion, mangos's too).
    if map.gameobject(object).is_some_and(|o| {
        o.loot
            .as_ref()
            .is_some_and(|l| l.looters.iter().any(|g| *g != caster))
    }) {
        send_failure(map, caster, s.id, FAIL_CHEST_IN_USE);
        return;
    }
    let lock = object_lock(&data, entry);
    let Some(p) = map.players.get(&caster) else {
        return;
    };
    let opening = match resolve_lock(&data, p, s, index, lock, cast_item) {
        Ok(o) => o,
        Err(code) => {
            send_failure(map, caster, s.id, code);
            return;
        }
    };
    if !attempt_takes(&opening) {
        send_failure(map, caster, s.id, FAIL_TRY_AGAIN);
        return;
    }
    // Once a spawn for each player (Thistle Tea's `skilled_players`).
    let skilled = map
        .loot_state
        .skilled
        .get(&object)
        .is_some_and(|who| who.contains(&caster));
    if !skilled && lock_skill_up(map, caster, &opening) {
        map.loot_state
            .skilled
            .entry(object)
            .or_default()
            .insert(caster);
    }
    opened(map, caster, object);
}

/// An object whose lock is open: a door opens, a chest or node shows its
/// loot, and its scripts run.
fn opened(map: &mut Map, caster: Guid, object: Guid) {
    let Some(kind) = map.gameobject(object).map(|o| o.kind) else {
        return;
    };
    match kind {
        GO_DOOR | GO_BUTTON => {
            if let Some(o) = map.gameobject_mut(object) {
                o.fields.set_u32(GAMEOBJECT_STATE, 0);
            }
            crate::game::map::scripts::object_used(map, caster, object);
        }
        // A battleground's banner or beacon.
        _ if crate::game::map::battleground::opened(map, caster, object) => {}
        // Disarm Trap: the trap is spent, with nothing to loot (mangos's
        // `SendLoot` for a trap, Thistle Tea's `deplete_trap`).
        crate::game::map::objects::GO_TRAP => crate::game::map::objects::disarm(map, object),
        _ => {
            loot::open_object(map, caster, object);
            crate::game::map::scripts::object_used(map, caster, object);
        }
    }
}

/// A lockbox picked (Thistle Tea's `unlock_item`): the attempt may fail,
/// as at a chest; picked, it is unlocked and opens at once, and the skill
/// may rise.
fn open_lock_item(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    item: Guid,
    cast_item: Option<u32>,
) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&caster) else {
        return;
    };
    if let Some(code) = lockbox_fails(&data, p, s, index, item, cast_item) {
        send_failure(map, caster, s.id, code);
        return;
    }
    let Some(i) = p.items.iter().position(|i| i.guid == item) else {
        return;
    };
    let (bag, slot, entry) = (p.items[i].bag, p.items[i].slot, p.items[i].entry);
    let lock = data.item(entry).map_or(0, |t| t.lock_id);
    let Ok(opening) = resolve_lock(&data, p, s, index, lock, cast_item) else {
        return;
    };
    if !attempt_takes(&opening) {
        send_failure(map, caster, s.id, FAIL_TRY_AGAIN);
        return;
    }
    if let Some(p) = map.players.get_mut(&caster)
        && let Some(i) = p.items.iter_mut().find(|i| i.guid == item)
    {
        let flags = i.fields.get_u32(ITEM_FIELD_FLAGS);
        i.fields
            .set_u32(ITEM_FIELD_FLAGS, flags | ITEM_FLAG_UNLOCKED);
    }
    lock_skill_up(map, caster, &opening);
    crate::game::map::items_misc::open_as(map, caster, bag, slot, LootKind::Opened);
}

/// Skinning a corpse: its skins, and the skill.
fn skin(map: &mut Map, caster: Guid, creature: Guid, spell: u32) {
    let data = map.realm.data.clone();
    let Some((entry, level)) = map
        .creature(creature)
        .filter(|c| {
            !c.alive() && has_skin(&data, c) && c.loot.as_ref().is_none_or(|l| l.is_empty())
        })
        .map(|c| (c.entry, c.fields.get_u32(UNIT_FIELD_LEVEL)))
    else {
        // Skinned by another first (Thistle Tea's check at completion).
        send_failure(map, caster, spell, FAIL_TARGET_UNSKINNABLE);
        return;
    };
    let Some(t) = data.creature_template(entry) else {
        return;
    };
    let Some(table) = data.loot.skinning.get(&t.skinning_loot_id) else {
        return;
    };
    // The attempt may fail: the skill rolled from 25 below to 37 above
    // must reach the corpse's need (Thistle Tea's `Skinning.attempt?`).
    let skill = map
        .players
        .get(&caster)
        .map_or(0, |p| skill_value(p, SKILL_SKINNING));
    let roll = skill as i32 - 25 + rand::rng().random_range(0..=62);
    if skill < 300 && skinning_required(level, skill) as i32 > roll {
        send_failure(map, caster, spell, FAIL_TRY_AGAIN);
        return;
    }
    let wanted = map
        .players
        .get(&caster)
        .map(|p| crate::game::map::quests::wanted_items(p, &data))
        .unwrap_or_default();
    let loot = loot::personal(&data, table, caster, &wanted, LootKind::Skinning);
    let elite = matches!(t.rank, 1..=3);
    // Skinning the Beast frees the hunter it swallowed.
    if entry == THE_BEAST {
        crate::game::map::spells::cast_triggered(map, caster, FINKLE_IS_EINHORN, Some(caster));
    }
    if let Some(c) = map.creature_mut(creature) {
        c.loot = Some(loot);
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SKINNABLE, false);
    }
    let learned = map
        .players
        .get(&caster)
        .and_then(|p| skills::value(p, SKILL_SKINNING))
        .unwrap_or(0);
    let required = if level < 20 {
        level.saturating_sub(10) * 10
    } else {
        level * 5
    };
    let chance = gather_gain_chance(SKILL_SKINNING, learned, required, if elite { 2 } else { 1 });
    try_raise(map, caster, SKILL_SKINNING, chance);
    loot::show(map, caster, creature);
}

/// Picking a living creature's pockets: once a life.
fn pick_pocket(map: &mut Map, caster: Guid, creature: Guid) {
    let data = map.realm.data.clone();
    let Some((entry, level)) = map
        .creature(creature)
        .filter(|c| c.alive())
        .map(|c| (c.entry, c.fields.get_u32(UNIT_FIELD_LEVEL)))
    else {
        return;
    };
    // Picked already this life, a pocket holds only the rogue's own quest
    // drops (Thistle Tea's `Pickpocket.session`).
    let picked_before = !map.loot_state.picked.insert(creature);
    let Some(t) = data.creature_template(entry) else {
        return;
    };
    let wanted = map
        .players
        .get(&caster)
        .map(|p| crate::game::map::quests::wanted_items(p, &data))
        .unwrap_or_default();
    let player_level = map.players.get(&caster).map_or(1, |p| u32::from(p.level()));
    let mut loot = match data.loot.pickpocket.get(&t.pickpocket_loot_id) {
        Some(table) => loot::personal(&data, table, caster, &wanted, LootKind::Pickpocket),
        None => Loot {
            owner: Some(caster),
            kind: LootKind::Pickpocket,
            ..Default::default()
        },
    };
    let mut rng = rand::rng();
    loot.gold = 10 * (rng.random_range(0..=level / 2) + rng.random_range(0..=player_level / 2));
    if picked_before {
        loot.gold = 0;
        loot.items.retain(|s| s.quest);
        if loot.items.is_empty() {
            return;
        }
    }
    if let Some(c) = map.creature_mut(creature) {
        c.loot = Some(loot);
    }
    loot::show(map, caster, creature);
}

/// Disenchanting an item: it goes, and its dust and shards are looted.
fn disenchant(map: &mut Map, caster: Guid, item: Guid, spell: u32) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&caster) else {
        return;
    };
    let Some(i) = p.items.iter().position(|x| x.guid == item) else {
        return;
    };
    let Some(t) = data.item(p.items[i].entry) else {
        return;
    };
    // Nothing to roll, or nothing rolled: try again, the item kept
    // (Thistle Tea's).
    let loot = data
        .loot
        .disenchant
        .get(&t.disenchant_id)
        .map(|table| loot::personal(&data, table, caster, &[], LootKind::Disenchant));
    let Some(loot) = loot.filter(|l| !l.items.is_empty()) else {
        send_failure(map, caster, spell, FAIL_TRY_AGAIN);
        return;
    };
    crate::game::map::inventory::take_item(map, caster, i, 1);
    let value = map
        .players
        .get(&caster)
        .and_then(|p| skills::value(p, SKILL_ENCHANTING))
        .unwrap_or(0);
    let chance = match value {
        60.. => 0,
        40.. => 25,
        20.. => 75,
        _ => 100,
    };
    try_raise(map, caster, SKILL_ENCHANTING, f64::from(chance));
    loot::show_item_loot(map, caster, item, loot);
}

/// Casting a line: a bobber in the water ahead, which a fish bites at
/// some point in the channel.
fn cast_line(map: &mut Map, caster: Guid, s: &SpellTemplate, index: usize) {
    let data = map.realm.data.clone();
    let entry = s.effects[index].misc_value.max(0) as u32;
    let Some(t) = data.gameobject_templates.get(&entry).cloned() else {
        return;
    };
    let Some(p) = map.players.get(&caster) else {
        return;
    };
    let (pos, o, zone, area) = (p.position(), p.movement.orientation, p.zone, p.area);
    // Where the cast found water; without a navmesh, ahead at the
    // player's height.
    let at = map
        .loot_state
        .fishing_spots
        .remove(&caster)
        .unwrap_or_else(|| line_reach(pos, o));
    // The zone and area are the water's (Thistle Tea's, mangos's).
    let (zone, area) = crate::game::map::world::zone_and_area(map, at).unwrap_or((zone, area));
    let mut rng = rand::rng();
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let mut go = GameObject::from_spawn(
        &t,
        &GameObjectSpawn {
            guid: counter | 0x80_0000,
            entry,
            map: map.key.map,
            x: at.x,
            y: at.y,
            z: at.z,
            orientation: o,
            state: 1,
            ..Default::default()
        },
    );
    go.spawn_guid = None;
    go.fields.set_guid(OBJECT_FIELD_CREATED_BY, caster);
    go.fields
        .set_u32(GAMEOBJECT_LEVEL, u32::from(map.players[&caster].level()));
    let g = map.add_gameobject(go);
    let now = map.now;
    let bite = Duration::from_millis(BITE_DELAYS_MS[rng.random_range(0..BITE_DELAYS_MS.len())]);
    // An old line comes out of the water.
    let old: Vec<Guid> = map
        .loot_state
        .bobbers
        .iter()
        .filter(|b| b.owner == caster)
        .map(|b| b.guid)
        .collect();
    for b in old {
        map.remove_gameobject(b);
    }
    map.loot_state.bobbers.retain(|b| b.owner != caster);
    map.loot_state.bobbers.push(Bobber {
        guid: g,
        owner: caster,
        spell: s.id,
        bite_at: now + bite,
        gone_at: now + bite + CATCH_WINDOW,
        bitten: false,
        zone,
        area,
    });
    // The channel lasts as long as the line is out (Thistle Tea's
    // `adjust_channel`).
    let lasts = bite + CATCH_WINDOW;
    if let Some(p) = map.players.get_mut(&caster) {
        p.fields.set_guid(UNIT_FIELD_CHANNEL_OBJECT, g);
        if let Some(c) = p.caster.casting.as_mut().filter(|c| c.spell == s.id) {
            c.ends = now + lasts;
            c.channel_until = Some(now + lasts);
            c.duration = lasts.as_millis() as u32;
        }
        p.outbound.send(&MSG_CHANNEL_UPDATE_Server {
            time: lasts.as_millis() as u32,
        });
    }
}

/// Where a line cast from `at` facing `o` lands: ten to twenty yards
/// ahead, a little to either side (Thistle Tea's `cast_position`).
fn line_reach(at: Vector3d, o: f32) -> Vector3d {
    let mut rng = rand::rng();
    let reach = 10.0 + rng.random_range(0.0f32..10.0);
    let angle = o + rng.random_range(-0.25f32..0.25);
    Vector3d {
        x: at.x + reach * angle.cos(),
        y: at.y + reach * angle.sin(),
        z: at.z,
    }
}

/// As a fishing cast begins: where its line will land, on the surface of
/// water a yard deep at least and in sight; else it isn't fishable
/// (Thistle Tea's `prepare_cast`). Without a navmesh to say, it lands.
pub fn fishing_spot(map: &mut Map, caster: Guid, s: &SpellTemplate) -> Option<u8> {
    let fishing =
        (0..3).any(|i| s.effects[i].effect == EFFECT_TRANS_DOOR && handles(&map.realm.data, s, i));
    if !fishing {
        return None;
    }
    let (pos, o) = map
        .players
        .get(&caster)
        .map(|p| (p.position(), p.movement.orientation))?;
    let nav = map.nav.as_mut()?;
    let to = line_reach(pos, o);
    let surface = nav.liquid_surface(Vector3d {
        z: pos.z + 1.0,
        ..to
    });
    let spot = surface.and_then(|surface| {
        let floor = nav.floor_under(to.x, to.y, surface).unwrap_or(surface);
        let at = Vector3d { z: surface, ..to };
        (surface - floor >= 1.0 && nav.line_of_sight(pos, at)).then_some(at)
    });
    match spot {
        Some(at) => {
            map.loot_state.fishing_spots.insert(caster, at);
            None
        }
        None => Some(FAIL_NOT_FISHABLE),
    }
}

/// A fishing channel ended, cancelled or run out: its line comes out of
/// the water, unhooked if its time was up (Thistle Tea's despawn and
/// `fishing_expire`).
pub fn line_ended(map: &mut Map, player: Guid) {
    let now = map.now;
    let lines: Vec<(Guid, bool)> = map
        .loot_state
        .bobbers
        .iter()
        .filter(|b| b.owner == player)
        .map(|b| (b.guid, b.gone_at <= now))
        .collect();
    for (g, timed_out) in lines {
        map.loot_state.bobbers.retain(|b| b.guid != g);
        map.remove_gameobject(g);
        if timed_out && let Some(p) = map.players.get(&player) {
            p.outbound.send(&SMSG_FISH_NOT_HOOKED {});
        }
    }
}

/// The fishing skill's chance to rise, in percent.
pub fn fishing_gain_chance(value: u32) -> u32 {
    if value < 75 {
        100
    } else {
        2500 / value.saturating_sub(50).max(1)
    }
}

/// Whether a catch is landed: the zone's skill and a roll of 1 to 100.
pub fn catch_landed(skill: u32, zone_skill: i32, roll: i32) -> bool {
    skill as i32 >= zone_skill && roll <= skill as i32 - zone_skill + 5
}

/// `CMSG_GAMEOBJ_USE` on something this module owns: a bobber. Whether it
/// was one.
pub fn use_object(map: &mut Map, player: Guid, object: Guid) -> bool {
    let Some(b) = map
        .loot_state
        .bobbers
        .iter()
        .find(|b| b.guid == object)
        .cloned()
    else {
        return false;
    };
    if b.owner != player {
        return true;
    }
    map.loot_state.bobbers.retain(|x| x.guid != object);
    end_channel(map, player);
    if !b.bitten {
        if let Some(p) = map.players.get(&player) {
            p.outbound.send(&SMSG_FISH_NOT_HOOKED {});
        }
        map.remove_gameobject(object);
        return true;
    }
    let data = map.realm.data.clone();
    let skill = map
        .players
        .get(&player)
        .map_or(0, |p| skill_value(p, SKILL_FISHING));
    let zone_skill = data
        .crafts
        .fishing_skill
        .get(&b.area)
        .or_else(|| data.crafts.fishing_skill.get(&b.zone))
        .copied()
        .unwrap_or(0);
    let roll = rand::rng().random_range(1..=100);
    if !catch_landed(skill, zone_skill, roll) {
        if let Some(p) = map.players.get(&player) {
            p.outbound.send(&SMSG_FISH_ESCAPED {});
        }
        map.remove_gameobject(object);
        return true;
    }
    let learned = map
        .players
        .get(&player)
        .and_then(|p| skills::value(p, SKILL_FISHING))
        .unwrap_or(0);
    try_raise(
        map,
        player,
        SKILL_FISHING,
        f64::from(fishing_gain_chance(learned)),
    );
    let wanted = map
        .players
        .get(&player)
        .map(|p| crate::game::map::quests::wanted_items(p, &data))
        .unwrap_or_default();
    let loot = match hole_near(map, object) {
        Some((hole, table)) => {
            let mut l = loot::personal(&data, &table, player, &wanted, LootKind::Fishing);
            l.gold = 0;
            // Between its least and most, rolled for the spawn (Thistle
            // Tea's, mangos's).
            let (least, most) = data
                .gameobject_templates
                .get(&map.gameobject(hole).map_or(0, |o| o.entry))
                .map_or((1, 1), |t| {
                    let least = t.data[2].max(1);
                    (least, t.data[3].max(least))
                });
            let max = *map
                .loot_state
                .hole_uses
                .entry(hole)
                .or_insert_with(|| rand::rng().random_range(least..=most));
            let uses = {
                let u = map.loot_state.node_uses.entry(hole).or_insert(0);
                *u += 1;
                *u
            };
            if uses >= max {
                despawn(map, hole);
            }
            l
        }
        None => {
            let table = data
                .loot
                .fishing
                .get(&b.area)
                .or_else(|| data.loot.fishing.get(&b.zone))
                .cloned()
                .unwrap_or_default();
            loot::personal(&data, &table, player, &wanted, LootKind::Fishing)
        }
    };
    if let Some(o) = map.gameobject_mut(object) {
        o.loot = Some(loot);
    }
    loot::show(map, player, object);
    true
}

/// A school of fish near the bobber, and its loot table.
fn hole_near(map: &Map, bobber: Guid) -> Option<(Guid, Vec<crate::game::data::LootEntry>)> {
    let data = &map.realm.data;
    let at = map.gameobject(bobber)?.position;
    map.grid
        .around(at.x, at.y, FISHING_HOLE_RANGE)
        .filter(|g| guid::is_game_object(*g) && *g != bobber)
        .filter_map(|g| map.gameobject(g))
        .find_map(|o| {
            let t = data.gameobject_templates.get(&o.entry)?;
            (t.kind == GO_FISHING_HOLE && dist(o.position, at) <= (t.data[0] as f32).max(1.0))
                .then(|| {
                    data.loot
                        .gameobject
                        .get(&t.data[1])
                        .map(|l| (o.guid, l.clone()))
                })
                .flatten()
        })
}

fn end_channel(map: &mut Map, player: Guid) {
    if let Some(p) = map.players.get_mut(&player) {
        p.caster.casting = None;
        p.fields.set_guid(UNIT_FIELD_CHANNEL_OBJECT, Guid::zero());
        p.fields.set_u32(UNIT_CHANNEL_SPELL, 0);
        p.outbound.send(&MSG_CHANNEL_UPDATE_Server { time: 0 });
    }
}

/// A corpse looted clean can be skinned, if its kind has skins.
pub fn corpse_looted(map: &mut Map, creature: Guid) {
    let data = map.realm.data.clone();
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    let skinnable = data
        .creature_template(c.entry)
        .is_some_and(|t| t.skinning_loot_id != 0);
    let skinned = c
        .loot
        .as_ref()
        .is_some_and(|l| l.kind == LootKind::Skinning);
    if skinnable && !skinned {
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SKINNABLE, true);
    }
}

/// An object emptied: a bobber goes; a gathered herb or vein goes, or
/// (a vein with uses left) fills again; a used-up chest goes. What goes
/// comes back on its spawn's timer.
pub fn object_looted(map: &mut Map, player: Guid, object: Guid) {
    let data = map.realm.data.clone();
    let Some((entry, kind)) = map.gameobject(object).map(|o| (o.entry, o.kind)) else {
        return;
    };
    if kind == GO_FISHING_NODE {
        map.remove_gameobject(object);
        return;
    }
    let Some(t) = data.gameobject_templates.get(&entry) else {
        return;
    };
    if kind != GO_CHEST {
        return;
    }
    let lock_type = data
        .crafts
        .locks
        .get(&t.data[0])
        .and_then(|keys| keys.iter().find(|k| k.kind == LOCK_KEY_SKILL))
        .map(|k| (k.index, k.skill));
    let gathered = lock_type.is_some_and(|(ty, _)| matches!(ty, 2 | 3));
    if !gathered {
        // Any other chest emptied goes until its spawn's time (Thistle
        // Tea's `Chest`).
        despawn(map, object);
        return;
    }
    let uses = {
        let u = map.loot_state.node_uses.entry(object).or_insert(0);
        *u += 1;
        *u
    };
    let (min, max) = (t.data[4], t.data[5]);
    let skill = lock_type
        .and_then(|(ty, _)| lock_skill(ty))
        .and_then(|sk| map.players.get(&player).map(|p| skill_value(p, sk)))
        .unwrap_or(0);
    let required = lock_type.map_or(0, |(_, r)| r);
    let roll = rand::rng().random_range(0.0f64..100.0);
    if replenish(uses, min, max, skill, required, roll) {
        if let Some(o) = map.gameobject_mut(object) {
            o.loot = None;
        }
        return;
    }
    despawn(map, object);
}

/// Whether a vein fills again for another gathering (Thistle Tea's
/// `Gathering.replenish?`).
pub fn replenish(uses: u32, min: u32, max: u32, skill: u32, required: u32, roll: f64) -> bool {
    if min == 0 || max <= min || uses >= max {
        return false;
    }
    if uses < min {
        return true;
    }
    roll < 100.0 * 0.8f64.powf(4.0 * f64::from(uses) / f64::from(max))
        + f64::from(skill) / f64::from(required + 25)
}

/// An object leaves the world until its spawn's respawn time is up.
fn despawn(map: &mut Map, object: Guid) {
    let data = map.realm.data.clone();
    map.loot_state.node_uses.remove(&object);
    map.loot_state.skilled.remove(&object);
    map.loot_state.hole_uses.remove(&object);
    let Some(o) = map.remove_gameobject(object) else {
        return;
    };
    let Some(spawn) = o.spawn_guid else {
        return;
    };
    let secs = data
        .gameobject_spawns
        .get(&map.key.map)
        .and_then(|l| l.iter().find(|s| s.guid == spawn))
        .map_or(300, |s| s.respawn_min.max(1));
    let after = Duration::from_secs(u64::from(secs));
    map.loot_state.respawns.push((map.now + after, spawn));
    crate::game::map::instance_script::chest_gone(map, spawn, after);
}

/// Bobbers bite and come out; gathered objects come back.
pub fn respawn_nodes(map: &mut Map) {
    let now = map.now;
    let bites: Vec<(Guid, Guid)> = map
        .loot_state
        .bobbers
        .iter_mut()
        .filter(|b| !b.bitten && b.bite_at <= now)
        .map(|b| {
            b.bitten = true;
            (b.guid, b.owner)
        })
        .collect();
    for (g, owner) in bites {
        if let Some(o) = map.gameobject_mut(g) {
            o.fields.set_u32(GAMEOBJECT_STATE, 0);
        }
        if let Some(frame) = crate::game::conn::encode(&SMSG_GAMEOBJECT_CUSTOM_ANIM {
            guid: g,
            animation_id: 0,
        }) {
            map.send_to_observers(g, &frame, false);
            if let Some(p) = map.players.get(&owner) {
                p.outbound.send_frame(frame);
            }
        }
        // The splash (Thistle Tea's bite, sound 3355).
        if let Some(frame) =
            crate::game::conn::encode(&wow_world_messages::vanilla::SMSG_PLAY_OBJECT_SOUND {
                sound_id: BITE_SOUND,
                guid: g,
            })
        {
            map.send_to_observers(g, &frame, false);
        }
    }
    let gone: Vec<(Guid, Guid)> = map
        .loot_state
        .bobbers
        .iter()
        .filter(|b| b.gone_at <= now)
        .map(|b| (b.guid, b.owner))
        .collect();
    for (g, owner) in gone {
        map.loot_state.bobbers.retain(|b| b.guid != g);
        map.remove_gameobject(g);
        if let Some(p) = map.players.get(&owner) {
            p.outbound.send(&SMSG_FISH_NOT_HOOKED {});
        }
        end_channel(map, owner);
    }
    if map.loot_state.respawns.is_empty() {
        return;
    }
    let due: Vec<u32> = map
        .loot_state
        .respawns
        .iter()
        .filter(|(t, _)| *t <= now)
        .map(|(_, s)| *s)
        .collect();
    if due.is_empty() {
        return;
    }
    map.loot_state.respawns.retain(|(t, _)| *t > now);
    let data = map.realm.data.clone();
    use crate::game::map::pools::{self, Member};
    for spawn in due {
        // A pooled node's pool picks again: it may come back as another.
        let Some(back) = pools::came_back(map, Member::Object(spawn)) else {
            let Some(s) = data
                .gameobject_spawns
                .get(&map.key.map)
                .and_then(|l| l.iter().find(|s| s.guid == spawn))
            else {
                continue;
            };
            if let Some(t) = data.gameobject_templates.get(&s.entry) {
                map.add_gameobject(GameObject::from_spawn(t, s));
            }
            continue;
        };
        for m in back {
            match m {
                Member::Object(s) => pools::spawn_object(map, s),
                Member::Creature(s) => crate::game::map::ai::spawn_creature_of(map, s),
                Member::Pool(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_ups_follow_the_colours() {
        assert_eq!(craft_gain_chance(10, 15, 35), 100);
        assert_eq!(craft_gain_chance(15, 15, 35), 75);
        assert_eq!(craft_gain_chance(25, 15, 35), 25);
        assert_eq!(craft_gain_chance(35, 15, 35), 0);
        assert_eq!(gather_gain_chance(SKILL_HERBALISM, 1, 0, 1), 100.0);
        assert_eq!(gather_gain_chance(SKILL_HERBALISM, 60, 0, 1), 25.0);
        assert_eq!(gather_gain_chance(SKILL_MINING, 80, 0, 1), 12.5);
        assert_eq!(fishing_gain_chance(150), 25);
        assert!(catch_landed(100, 0, 100));
        assert!(!catch_landed(30, 50, 1));
        assert_eq!(skinning_required(30, 150), 150);
        assert_eq!(skinning_required(15, 1), 50);
        assert!(replenish(1, 2, 5, 100, 75, 99.0));
        assert!(!replenish(5, 2, 5, 100, 75, 0.0));
    }
}
