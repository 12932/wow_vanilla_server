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

/// The skills recipes raise.
const CRAFTS: &[u32] = &[
    SKILL_FIRST_AID,
    SKILL_BLACKSMITHING,
    SKILL_LEATHERWORKING,
    SKILL_ALCHEMY,
    SKILL_COOKING,
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

// `SpellCastResult`s.
const FAIL_BAD_TARGETS: u8 = 0x0a;
const FAIL_CANT_BE_DISENCHANTED: u8 = 0x0c;
const FAIL_CHEST_IN_USE: u8 = 0x15;
const FAIL_ITEM_NOT_FOUND: u8 = 0x27;
const FAIL_LOW_CASTLEVEL: u8 = 0x2c;
const FAIL_ONLY_STEALTHED: u8 = 0x57;
pub const FAIL_REQUIRES_SPELL_FOCUS: u8 = 0x5e;
const FAIL_TARGETS_DEAD: u8 = 0x65;
const FAIL_TARGET_NOT_DEAD: u8 = 0x6e;
const FAIL_TARGET_NOT_LOOTED: u8 = 0x70;
const FAIL_TARGET_NO_POCKETS: u8 = 0x72;
const FAIL_TARGET_UNSKINNABLE: u8 = 0x74;
const FAIL_TOTEMS: u8 = 0x78;
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

/// A game object's lock.
fn object_lock(data: &GameData, entry: u32) -> u32 {
    data.gameobject_templates
        .get(&entry)
        .map(|t| match t.kind {
            GO_DOOR | GO_BUTTON => t.data[1],
            GO_CHEST | GO_GOOBER => t.data[0],
            _ => 0,
        })
        .unwrap_or(0)
}

/// What opening a lock takes: (skill, its value with the spell's bonus,
/// the value needed), or an error.
pub fn resolve_lock(
    data: &GameData,
    p: &Player,
    s: &SpellTemplate,
    index: usize,
    lock: u32,
) -> Result<(Option<u32>, u32, u32), u8> {
    let Some(keys) = data.crafts.locks.get(&lock) else {
        return Ok((None, 0, 0));
    };
    let lock_type = s.effects[index].misc_value.max(0) as u32;
    let bonus = crate::game::map::spells::points(s, index, 1).max(0) as u32;
    let mut failed = FAIL_BAD_TARGETS;
    for k in keys {
        match k.kind {
            LOCK_KEY_ITEM if k.index != 0 && p.count_of(k.index) > 0 => return Ok((None, 0, 0)),
            LOCK_KEY_SKILL if k.index == lock_type => {
                let skill = lock_skill(lock_type);
                let value = skill.map_or(0, |sk| skill_value(p, sk)) + bonus;
                let required = if skill.is_some() { k.skill } else { 0 };
                if value >= required {
                    return Ok((skill, value, required));
                }
                failed = FAIL_LOW_CASTLEVEL;
            }
            _ => {}
        }
    }
    Err(failed)
}

/// The skill a creature's skin needs.
pub fn skinning_required(level: u32, skill: u32) -> u32 {
    if skill < 100 {
        level.saturating_sub(10) * 10
    } else {
        level * 5
    }
}

/// A gathering skill's chance to rise, in percent (Thistle Tea's
/// `Gathering.gain_chance`).
pub fn gather_gain_chance(skill: u32, value: u32, required: u32, multiplier: u32) -> u32 {
    let chance = if value >= required + 100 {
        0
    } else if value >= required + 50 {
        25
    } else if value >= required + 25 {
        75
    } else {
        100
    };
    let divisor = if matches!(skill, SKILL_MINING | SKILL_SKINNING) {
        1 << (value / 75).min(8)
    } else {
        1
    };
    (chance * multiplier / divisor).min(100)
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

/// Raise `skill` by one, `chance` percent of the time.
pub fn try_raise(map: &mut Map, player: Guid, skill: u32, chance: u32) -> bool {
    let roll = rand::rng().random_range(0..100);
    roll < chance && map.players.get_mut(&player).is_some_and(|p| skills::raise(p, skill, 1))
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

/// Why `caster` can't cast `s` at `t` for its profession's sake: a spell
/// focus not near, tools not carried, a lock beyond the skill, a target
/// that can't be skinned, picked or disenchanted.
pub fn check(map: &Map, caster: Guid, s: &SpellTemplate, t: &Targets) -> Option<u8> {
    let data = &map.realm.data;
    let p = map.players.get(&caster)?;
    if s.requires_spell_focus != 0 && !focus_near(map, p.position(), s.requires_spell_focus) {
        return Some(FAIL_REQUIRES_SPELL_FOCUS);
    }
    if s.totems.iter().any(|t| *t != 0 && p.count_of(*t) == 0) {
        return Some(FAIL_TOTEMS);
    }
    for (index, e) in s.effects.iter().enumerate() {
        match e.effect {
            EFFECT_OPEN_LOCK => {
                let Some(object) = t.object else {
                    continue;
                };
                let o = map.gameobject(object)?;
                if o.loot.as_ref().is_some_and(|l| !l.looters.is_empty()) {
                    return Some(FAIL_CHEST_IN_USE);
                }
                let lock = object_lock(data, o.entry);
                if let Err(code) = resolve_lock(data, p, s, index, lock) {
                    return Some(code);
                }
            }
            EFFECT_OPEN_LOCK_ITEM => {
                let item = t.item.and_then(|g| p.items.iter().find(|i| i.guid == g));
                let Some(item) = item else {
                    return Some(FAIL_ITEM_NOT_FOUND);
                };
                let lock = data.item(item.entry).map_or(0, |t| t.lock_id);
                if lock == 0 {
                    return Some(FAIL_BAD_TARGETS);
                }
                if let Err(code) = resolve_lock(data, p, s, index, lock) {
                    return Some(code);
                }
            }
            EFFECT_SKINNING => {
                let c = t.unit.and_then(|u| map.creature(u));
                let Some(c) = c else {
                    return Some(FAIL_BAD_TARGETS);
                };
                if c.alive() {
                    return Some(FAIL_TARGET_NOT_DEAD);
                }
                if !c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SKINNABLE) {
                    let looted = c.loot.as_ref().is_none_or(|l| l.is_empty());
                    return Some(if looted {
                        FAIL_TARGET_UNSKINNABLE
                    } else {
                        FAIL_TARGET_NOT_LOOTED
                    });
                }
                let level = c.fields.get_u32(UNIT_FIELD_LEVEL);
                let skill = skill_value(p, SKILL_SKINNING);
                if skill == 0 || skinning_required(level, skill) > skill {
                    return Some(FAIL_LOW_CASTLEVEL);
                }
            }
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
                if !pockets || map.loot_state.picked.contains(&c.guid) {
                    return Some(FAIL_TARGET_NO_POCKETS);
                }
            }
            EFFECT_DISENCHANT => {
                let item = t.item.and_then(|g| p.items.iter().find(|i| i.guid == g));
                let ok = item.is_some_and(|i| {
                    i.count() == 1
                        && data.item(i.entry).is_some_and(|t| {
                            t.disenchant_id != 0 && t.flags & ITEM_FLAG_NO_DISENCHANT == 0
                        })
                });
                if !ok || skill_value(p, SKILL_ENCHANTING) == 0 {
                    return Some(FAIL_CANT_BE_DISENCHANTED);
                }
            }
            EFFECT_ENCHANT_ITEM | EFFECT_ENCHANT_ITEM_TEMPORARY => {
                let item = t.item.and_then(|g| p.items.iter().find(|i| i.guid == g));
                let Some(item) = item else {
                    return Some(FAIL_ITEM_NOT_FOUND);
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

/// A spell focus of `focus` within its reach of `at`.
fn focus_near(map: &Map, at: Vector3d, focus: u32) -> bool {
    let data = &map.realm.data;
    map.grid
        .around(at.x, at.y, 30.0)
        .filter(|g| guid::is_game_object(*g))
        .filter_map(|g| map.gameobject(g))
        .any(|o| {
            data.gameobject_templates.get(&o.entry).is_some_and(|t| {
                t.kind == GO_SPELL_FOCUS
                    && t.data[0] == focus
                    && dist(o.position, at) <= (t.data[1] as f32).max(5.0)
            })
        })
}

/// Run `s`'s profession effects at `t`; the effects run are returned as a
/// mask, and a recipe raises its skill.
pub fn execute(map: &mut Map, caster: Guid, s: &SpellTemplate, t: &Targets) -> u8 {
    let data = map.realm.data.clone();
    let mut done = 0u8;
    for index in 0..3 {
        if s.effects[index].effect == 0 || !handles(&data, s, index) {
            continue;
        }
        done |= 1 << index;
        match s.effects[index].effect {
            EFFECT_OPEN_LOCK => {
                if let Some(object) = t.object {
                    open_lock(map, caster, s, index, object);
                }
            }
            EFFECT_OPEN_LOCK_ITEM => {
                if let Some(item) = t.item {
                    open_lock_item(map, caster, s, index, item);
                }
            }
            EFFECT_SKINNING => {
                if let Some(unit) = t.unit {
                    skin(map, caster, unit);
                }
            }
            EFFECT_PICKPOCKET => {
                if let Some(unit) = t.unit {
                    pick_pocket(map, caster, unit);
                }
            }
            EFFECT_DISENCHANT => {
                if let Some(item) = t.item {
                    disenchant(map, caster, item);
                }
            }
            EFFECT_ENCHANT_ITEM | EFFECT_ENCHANT_ITEM_TEMPORARY => {
                if let Some(item) = t.item {
                    let id = s.effects[index].misc_value.max(0) as u32;
                    let temporary = (s.effects[index].effect == EFFECT_ENCHANT_ITEM_TEMPORARY).then(|| {
                        let secs = crate::game::map::spells::points(s, index, 1).max(1) as u32;
                        let charges = data.crafts.enchant_charges.get(&s.id).copied().unwrap_or(0);
                        (secs * 1000, charges)
                    });
                    crate::game::map::enchant::enchant_item(map, caster, item, id, temporary);
                }
            }
            EFFECT_TRANS_DOOR => cast_line(map, caster, s, index),
            _ => {}
        }
    }
    recipe_skill_up(map, caster, s);
    done
}

/// A recipe crafted: its skill may rise, by the recipe's colour.
pub fn recipe_skill_up(map: &mut Map, caster: Guid, s: &SpellTemplate) {
    let data = map.realm.data.clone();
    let Some(abilities) = data.spells.skill_abilities.get(&s.id) else {
        return;
    };
    for a in abilities.iter().filter(|a| CRAFTS.contains(&a.skill) && a.trivial_high > 0) {
        let Some(value) = map.players.get(&caster).and_then(|p| skills::value(p, a.skill)) else {
            continue;
        };
        let chance = craft_gain_chance(value, a.trivial_low, a.trivial_high);
        try_raise(map, caster, a.skill, chance);
    }
}

/// Gathering, lockpicking or a plain opening: a lock opened with skill
/// may fail, and may raise the skill; a chest or node then shows its loot,
/// a door opens.
fn open_lock(map: &mut Map, caster: Guid, s: &SpellTemplate, index: usize, object: Guid) {
    let data = map.realm.data.clone();
    let Some((entry, kind)) = map.gameobject(object).map(|o| (o.entry, o.kind)) else {
        return;
    };
    let lock = object_lock(&data, entry);
    let Some(p) = map.players.get(&caster) else {
        return;
    };
    let (skill, value, required) = match resolve_lock(&data, p, s, index, lock) {
        Ok(r) => r,
        Err(code) => {
            send_failure(map, caster, s.id, code);
            return;
        }
    };
    if let Some(skill) = skill.filter(|sk| matches!(*sk, SKILL_HERBALISM | SKILL_MINING | SKILL_LOCKPICKING)) {
        let roll = value as i32 - 26 + rand::rng().random_range(1..=63);
        let sure = matches!(skill, SKILL_HERBALISM | SKILL_MINING) && value >= 300;
        if !sure && (required as i32) > roll {
            send_failure(map, caster, s.id, FAIL_TRY_AGAIN);
            return;
        }
        let learned = map
            .players
            .get(&caster)
            .and_then(|p| skills::value(p, skill))
            .unwrap_or(0);
        try_raise(
            map,
            caster,
            skill,
            gather_gain_chance(skill, learned, required, 1),
        );
    }
    match kind {
        GO_DOOR | GO_BUTTON => {
            if let Some(o) = map.gameobject_mut(object) {
                o.fields.set_u32(GAMEOBJECT_STATE, 0);
            }
        }
        // A battleground's banner or beacon.
        _ if crate::game::map::battleground::opened(map, caster, object) => {}
        _ => loot::open_object(map, caster, object),
    }
}

/// A lockbox picked: it can be opened now.
fn open_lock_item(map: &mut Map, caster: Guid, s: &SpellTemplate, index: usize, item: Guid) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&caster) else {
        return;
    };
    let Some(entry) = p.items.iter().find(|i| i.guid == item).map(|i| i.entry) else {
        return;
    };
    let lock = data.item(entry).map_or(0, |t| t.lock_id);
    let (skill, _value, required) = match resolve_lock(&data, p, s, index, lock) {
        Ok(r) => r,
        Err(code) => {
            send_failure(map, caster, s.id, code);
            return;
        }
    };
    if let Some(p) = map.players.get_mut(&caster)
        && let Some(i) = p.items.iter_mut().find(|i| i.guid == item)
    {
        let flags = i.fields.get_u32(ITEM_FIELD_FLAGS);
        i.fields.set_u32(ITEM_FIELD_FLAGS, flags | ITEM_FLAG_UNLOCKED);
    }
    if let Some(skill) = skill {
        let learned = map
            .players
            .get(&caster)
            .and_then(|p| skills::value(p, skill))
            .unwrap_or(0);
        try_raise(map, caster, skill, gather_gain_chance(skill, learned, required, 1));
    }
}

/// Skinning a corpse: its skins, and the skill.
fn skin(map: &mut Map, caster: Guid, creature: Guid) {
    let data = map.realm.data.clone();
    let Some((entry, level)) = map
        .creature(creature)
        .filter(|c| c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SKINNABLE))
        .map(|c| (c.entry, c.fields.get_u32(UNIT_FIELD_LEVEL)))
    else {
        return;
    };
    let Some(t) = data.creature_template(entry) else {
        return;
    };
    let Some(table) = data.loot.skinning.get(&t.skinning_loot_id) else {
        return;
    };
    let wanted = map
        .players
        .get(&caster)
        .map(|p| crate::game::map::quests::wanted_items(p, &data))
        .unwrap_or_default();
    let loot = loot::personal(&data, table, caster, &wanted, LootKind::Skinning);
    let elite = matches!(t.rank, 1 | 2);
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
    if !map.loot_state.picked.insert(creature) {
        return;
    }
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
    if let Some(c) = map.creature_mut(creature) {
        c.loot = Some(loot);
    }
    loot::show(map, caster, creature);
}

/// Disenchanting an item: it goes, and its dust and shards are looted.
fn disenchant(map: &mut Map, caster: Guid, item: Guid) {
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
    let Some(table) = data.loot.disenchant.get(&t.disenchant_id) else {
        return;
    };
    let loot = loot::personal(&data, table, caster, &[], LootKind::Disenchant);
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
    try_raise(map, caster, SKILL_ENCHANTING, chance);
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
    let mut rng = rand::rng();
    let reach = 10.0 + rng.random_range(0.0f32..10.0);
    let angle = o + rng.random_range(-0.25f32..0.25);
    let at = Vector3d {
        x: pos.x + reach * angle.cos(),
        y: pos.y + reach * angle.sin(),
        z: pos.z,
    };
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
    if let Some(p) = map.players.get_mut(&caster) {
        p.fields.set_guid(UNIT_FIELD_CHANNEL_OBJECT, g);
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
    let Some(b) = map.loot_state.bobbers.iter().find(|b| b.guid == object).cloned() else {
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
    try_raise(map, player, SKILL_FISHING, fishing_gain_chance(learned));
    let wanted = map
        .players
        .get(&player)
        .map(|p| crate::game::map::quests::wanted_items(p, &data))
        .unwrap_or_default();
    let loot = match hole_near(map, object) {
        Some((hole, table)) => {
            let mut l = loot::personal(&data, &table, player, &wanted, LootKind::Fishing);
            l.gold = 0;
            let max = data
                .gameobject_templates
                .get(&map.gameobject(hole).map_or(0, |o| o.entry))
                .map_or(1, |t| t.data[3].max(t.data[2]).max(1));
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
                .then(|| data.loot.gameobject.get(&t.data[1]).map(|l| (o.guid, l.clone())))
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
    let skinned = c.loot.as_ref().is_some_and(|l| l.kind == LootKind::Skinning);
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
    let consumable = t.data[3] != 0;
    if !gathered && !consumable {
        // An ordinary chest fills again for the next one.
        if let Some(o) = map.gameobject_mut(object) {
            o.loot = None;
        }
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
    let at = map.now + Duration::from_secs(u64::from(secs));
    map.loot_state.respawns.push((at, spawn));
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
    for spawn in due {
        // A pooled node may come back as another of its pool.
        let waiting: Vec<u32> = map.loot_state.respawns.iter().map(|(_, s)| *s).collect();
        let spawn = crate::game::map::creature_groups::object_respawn_choice(map, spawn, &waiting);
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
        assert_eq!(gather_gain_chance(SKILL_HERBALISM, 1, 0, 1), 100);
        assert_eq!(gather_gain_chance(SKILL_HERBALISM, 60, 0, 1), 25);
        assert_eq!(gather_gain_chance(SKILL_MINING, 80, 0, 1), 12);
        assert_eq!(fishing_gain_chance(150), 25);
        assert!(catch_landed(100, 0, 100));
        assert!(!catch_landed(30, 50, 1));
        assert_eq!(skinning_required(30, 150), 150);
        assert_eq!(skinning_required(15, 1), 50);
        assert!(replenish(1, 2, 5, 100, 75, 99.0));
        assert!(!replenish(5, 2, 5, 100, 75, 0.0));
    }
}
