//! Enchantments on items: permanent ones (enchanters' work), temporary
//! ones (poisons, stones, oils, shamans' imbues) with a duration and
//! charges, and the random properties ("of the Bear") an item may roll.
//! What they give comes through the wearer's stats ([`add_to_gear`]),
//! auras ([`equip_spells`]) and weapon procs ([`weapon_procs`]); item
//! sets give their spells at each count worn.
//!
//! An item's enchantments are kept in its update fields and, for the
//! store, in its `data` string.

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::SMSG_ITEM_ENCHANT_TIME_UPDATE;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::derive::Gear;
use crate::game::map::player::{Item, Player};
use crate::game::store::BAG_PLAYER;

/// Enchantment slots.
pub const PERM_SLOT: usize = 0;
pub const TEMP_SLOT: usize = 1;
pub const PROP_SLOT_0: usize = 3;
pub const SLOTS: usize = 7;

/// `SpellItemEnchantment` effect types.
pub const TYPE_COMBAT_SPELL: u32 = 1;
pub const TYPE_DAMAGE: u32 = 2;
pub const TYPE_EQUIP_SPELL: u32 = 3;
pub const TYPE_RESISTANCE: u32 = 4;
pub const TYPE_STAT: u32 = 5;
pub const TYPE_TOTEM: u32 = 6;

/// One enchantment on an item: its id, the time it has left (0 for
/// lasting) and its charges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Enchant {
    pub id: u32,
    pub duration_ms: u32,
    pub charges: u32,
}

fn field(slot: usize) -> u16 {
    ITEM_FIELD_ENCHANTMENT + (slot as u16) * 3
}

pub fn get(item: &Item, slot: usize) -> Enchant {
    let f = field(slot);
    Enchant {
        id: item.fields.get_u32(f),
        duration_ms: item.fields.get_u32(f + 1),
        charges: item.fields.get_u32(f + 2),
    }
}

pub fn set(item: &mut Item, slot: usize, e: Enchant) {
    let f = field(slot);
    item.fields.set_u32(f, e.id);
    item.fields.set_u32(f + 1, e.duration_ms);
    item.fields.set_u32(f + 2, e.charges);
}

/// The item's enchantments with an id.
pub fn active(item: &Item) -> impl Iterator<Item = (usize, Enchant)> + '_ {
    (0..SLOTS)
        .map(|s| (s, get(item, s)))
        .filter(|(_, e)| e.id != 0)
}

/// An item's random property, rolled from its template's pool.
pub fn roll_random_property(data: &GameData, entry: u32, rng: &mut impl rand::Rng) -> u32 {
    let Some(pool) = data
        .item(entry)
        .map(|t| t.random_property)
        .filter(|p| *p != 0)
        .and_then(|p| data.crafts.random_pools.get(&p))
    else {
        return 0;
    };
    select_property(pool, rng.random_range(0.0..=1.0)).unwrap_or(0)
}

/// The property a `roll` from 0 to 1 picks of `pool`'s, by their chances
/// made to sum to one, a chance out of mangos's range (above 0.000001, at
/// most 100) left out (Thistle Tea's `ItemProperties.select`, mangos's
/// `LoadRandomEnchantmentsTable`); none of an empty pool.
pub fn select_property(pool: &[(u32, f32)], roll: f32) -> Option<u32> {
    let valid: Vec<(u32, f32)> = pool
        .iter()
        .copied()
        .filter(|(_, c)| *c > 0.000_001 && *c <= 100.0)
        .collect();
    let total: f32 = valid.iter().map(|(_, c)| c).sum();
    if total <= 0.0 {
        return None;
    }
    let mut at = 0.0;
    for (id, c) in &valid {
        at += c / total;
        if roll <= at {
            return Some(*id);
        }
    }
    valid.last().map(|(id, _)| *id)
}

/// Give an item its random property: the id the client names it by, and
/// the property's enchantments.
pub fn set_random_property(data: &GameData, item: &mut Item, property: u32) {
    item.fields
        .set_u32(ITEM_FIELD_RANDOM_PROPERTIES_ID, property);
    if let Some(enchants) = data.crafts.random_properties.get(&property) {
        for (i, id) in enchants.iter().take(SLOTS - PROP_SLOT_0).enumerate() {
            set(
                item,
                PROP_SLOT_0 + i,
                Enchant {
                    id: *id,
                    ..Default::default()
                },
            );
        }
    }
}

/// The item's enchantments and random property, from its stored `data`:
/// `text:N`, `rp:N` and `eS:ID/MS/CHARGES` tokens, `;` apart.
pub fn decode(data: &GameData, item: &mut Item) {
    let stored = item.data.clone();
    for token in stored.split(';') {
        if let Some(rp) = token.strip_prefix("rp:").and_then(|v| v.parse().ok()) {
            set_random_property(data, item, rp);
        } else if let Some(left) = token.strip_prefix("d:").and_then(|v| v.parse().ok()) {
            item.fields.set_u32(ITEM_FIELD_DURATION, left);
        } else if let Some(rest) = token.strip_prefix('e')
            && let Some((slot, values)) = rest.split_once(':')
            && let Ok(slot) = slot.parse::<usize>()
            && slot < SLOTS
        {
            let v: Vec<u32> = values.split('/').filter_map(|x| x.parse().ok()).collect();
            if let [id, ms, charges] = v[..] {
                set(
                    item,
                    slot,
                    Enchant {
                        id,
                        duration_ms: ms,
                        charges,
                    },
                );
            }
        }
    }
}

/// The item's `data` for the store: its text, what it wraps if a gift,
/// what is left in it if opened, random property and enchantments.
pub fn encode(item: &Item) -> String {
    let mut out: Vec<String> = item
        .data
        .split(';')
        .filter(|t| {
            t.starts_with("text:")
                || t.starts_with("gift:")
                || t.starts_with(crate::game::map::items_misc::LOOT_PREFIX)
        })
        .map(str::to_owned)
        .collect();
    let rp = item.fields.get_u32(ITEM_FIELD_RANDOM_PROPERTIES_ID);
    if rp != 0 {
        out.push(format!("rp:{rp}"));
    }
    // A timed item's time left.
    let left = item.fields.get_u32(ITEM_FIELD_DURATION);
    if left != 0 {
        out.push(format!("d:{left}"));
    }
    for (slot, e) in active(item) {
        // A random property's enchantments come with it.
        if slot >= PROP_SLOT_0 && rp != 0 {
            continue;
        }
        out.push(format!("e{slot}:{}/{}/{}", e.id, e.duration_ms, e.charges));
    }
    out.join(";")
}

/// What an equipped item's enchantments add to its wearer's gear: stats,
/// resistances and weapon damage.
pub fn add_to_gear(data: &GameData, item: &Item, g: &mut Gear) {
    let delay = data.item(item.entry).map_or(0, |t| t.delay);
    for (_, e) in active(item) {
        let Some(en) = data.spells.enchantments.get(&e.id) else {
            continue;
        };
        for (kind, min, _max, arg) in en.effects {
            match kind {
                TYPE_DAMAGE => add_weapon_damage(g, item, min as f32),
                // Rockbiter: damage a second, by the weapon's speed.
                TYPE_TOTEM => add_weapon_damage(g, item, min as f32 * delay as f32 / 1000.0),
                TYPE_RESISTANCE => {
                    let i = (arg as usize).min(6);
                    g.resistances[i] += min;
                }
                TYPE_STAT => match arg {
                    0 => g.mana += min,
                    1 => g.health += min,
                    3 => g.stats[1] += min,
                    4 => g.stats[0] += min,
                    5 => g.stats[3] += min,
                    6 => g.stats[4] += min,
                    7 => g.stats[2] += min,
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn add_weapon_damage(g: &mut Gear, item: &Item, amount: f32) {
    let w = match item.slot {
        15 => g.main.as_mut(),
        16 => g.off.as_mut(),
        17 => g.ranged.as_mut(),
        _ => None,
    };
    if let Some(w) = w {
        w.min += amount;
        w.max += amount;
    }
}

/// The spells an item's enchantments give while it is worn.
pub fn equip_spells(data: &GameData, item: &Item) -> Vec<u32> {
    active(item)
        .filter_map(|(_, e)| data.spells.enchantments.get(&e.id))
        .flat_map(|en| en.effects)
        .filter(|(kind, _, _, arg)| *kind == TYPE_EQUIP_SPELL && *arg != 0)
        .map(|(_, _, _, arg)| arg)
        .collect()
}

/// The item sets' spells a player's gear gives: each spell whose count is
/// worn, held by one of the set's pieces.
pub fn set_spells(data: &GameData, p: &Player) -> Vec<(Guid, u32)> {
    let mut counts: ahash::AHashMap<u32, (u32, Guid)> = ahash::AHashMap::new();
    for item in p
        .items
        .iter()
        .filter(|i| i.bag == BAG_PLAYER && i.slot < 19)
    {
        let Some(set) = data.item(item.entry).map(|t| t.set_id).filter(|s| *s != 0) else {
            continue;
        };
        let e = counts.entry(set).or_insert((0, item.guid));
        e.0 += 1;
    }
    let mut out = Vec::new();
    for (set, (worn, holder)) in counts {
        let Some(s) = data.crafts.item_sets.get(&set) else {
            continue;
        };
        // A set that asks a skill gives nothing short of it (Thistle Tea's
        // `EquipmentSets.qualified?`).
        if s.required_skill != 0
            && crate::game::map::professions::skill_value(p, s.required_skill)
                < s.required_skill_rank.max(1)
        {
            continue;
        }
        for (spell, needed) in &s.spells {
            if worn >= *needed {
                out.push((holder, *spell));
            }
        }
    }
    out
}

/// Put enchantment `id` on `item` (a guid in `player`'s items), in the
/// permanent or temporary slot.
pub fn enchant_item(
    map: &mut Map,
    player: Guid,
    item: Guid,
    id: u32,
    temporary: Option<(u32, u32)>,
) -> bool {
    let Some(p) = map.players.get_mut(&player) else {
        return false;
    };
    let Some(i) = p.items.iter().position(|x| x.guid == item) else {
        return false;
    };
    let (slot, e) = match temporary {
        Some((ms, charges)) => (
            TEMP_SLOT,
            Enchant {
                id,
                duration_ms: ms,
                charges,
            },
        ),
        None => (
            PERM_SLOT,
            Enchant {
                id,
                ..Default::default()
            },
        ),
    };
    set(&mut p.items[i], slot, e);
    // An enchanted item is bound to its owner.
    let flags = p.items[i].fields.get_u32(ITEM_FIELD_FLAGS);
    p.items[i].fields.set_u32(ITEM_FIELD_FLAGS, flags | 0x1);
    let worn = p.items[i].bag == BAG_PLAYER && p.items[i].slot < 19;
    if let Some((ms, _)) = temporary {
        p.outbound.send(&SMSG_ITEM_ENCHANT_TIME_UPDATE {
            item,
            slot: TEMP_SLOT as u32,
            duration: ms / 1000,
            player,
        });
    }
    if worn {
        let data = map.realm.data.clone();
        if let Some(p) = map.players.get_mut(&player) {
            // Its glow for others to see too.
            p.equip_fields();
            p.apply_stats(&data);
            p.gear_dirty = true;
        }
    }
    true
}

/// Take the enchantment in `slot` off `item`.
pub fn clear(map: &mut Map, player: Guid, item: Guid, slot: usize) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some(i) = p.items.iter().position(|x| x.guid == item) else {
        return;
    };
    set(&mut p.items[i], slot, Enchant::default());
    if slot == TEMP_SLOT {
        p.outbound.send(&SMSG_ITEM_ENCHANT_TIME_UPDATE {
            item,
            slot: TEMP_SLOT as u32,
            duration: 0,
            player,
        });
    }
    if p.items[i].bag == BAG_PLAYER && p.items[i].slot < 19 {
        p.equip_fields();
        p.apply_stats(&data);
        p.gear_dirty = true;
    }
}

/// How often temporary enchantments' time runs down.
const TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// Temporary enchantments run down while their owner is in the world, and
/// go when their time is up.
pub fn update(map: &mut Map) {
    let now = map.now;
    let last = *map.loot_state.enchant_tick_at.get_or_insert(now);
    let elapsed = now.duration_since(last);
    if elapsed < TICK {
        return;
    }
    map.loot_state.enchant_tick_at = Some(now);
    let ms = elapsed.as_millis() as u32;
    let mut expired = Vec::new();
    for p in map.players.values_mut() {
        for item in p.items.iter_mut() {
            let e = get(item, TEMP_SLOT);
            if e.id == 0 || e.duration_ms == 0 {
                continue;
            }
            let left = e.duration_ms.saturating_sub(ms);
            if left == 0 {
                expired.push((p.guid, item.guid));
            } else {
                item.fields.set_u32(field(TEMP_SLOT) + 1, left);
            }
        }
    }
    for (player, item) in expired {
        clear(map, player, item, TEMP_SLOT);
    }
}

/// A weapon's procs for a hit: its enchantments' combat spells (with their
/// chance, or procs a minute), and its own chance-on-hit spells. A
/// temporary enchantment spends a charge each time; with none left it
/// goes.
pub fn weapon_procs(map: &mut Map, e: &crate::game::map::procs::Event) {
    use crate::game::map::procs::*;
    const EFFECT_ADD_EXTRA_ATTACKS: u32 = 19;
    let data = map.realm.data.clone();
    if e.actor_flags
        & (PROC_DONE_MELEE
            | PROC_DONE_OFFHAND
            | PROC_DONE_RANGED
            | PROC_DONE_MELEE_SPELL
            | PROC_DONE_RANGED_SPELL)
        == 0
        || e.ex & (EX_NORMAL_HIT | EX_CRITICAL_HIT | EX_BLOCK) == 0
        || e.actor == e.target
        || !map.unit_view(e.target).is_some_and(|u| u.alive)
    {
        return;
    }
    // Abilities with the weapon proc it too; spells don't.
    if let Some(s) = e.spell.and_then(|id| data.spells.get(id))
        && s.equipped_item_class != 2
    {
        return;
    }
    let slot = if e.offhand {
        16
    } else if e.ranged || e.actor_flags & (PROC_DONE_RANGED | PROC_DONE_RANGED_SPELL) != 0 {
        17
    } else {
        15
    };
    let Some(p) = map.players.get(&e.actor) else {
        return;
    };
    let Some(item) = p
        .items
        .iter()
        .find(|i| i.bag == BAG_PLAYER && i.slot == slot)
    else {
        return;
    };
    // A broken weapon isn't struck with (mangos's `GetWeaponForAttack` of
    // a usable one): its enchantments and spells don't go off, nor spend
    // a charge.
    if item.fields.get_u32(ITEM_FIELD_MAXDURABILITY) > 0
        && item.fields.get_u32(ITEM_FIELD_DURABILITY) == 0
    {
        return;
    }
    let item_guid = item.guid;
    let delay = data.item(item.entry).map_or(2000, |t| t.delay.max(1));
    // An extra attack doesn't bring more extra attacks.
    let wanted = |spell: u32| {
        !(e.extra_attack
            && data
                .spells
                .get(spell)
                .is_some_and(|s| s.has_effect(EFFECT_ADD_EXTRA_ATTACKS)))
    };
    let mut rng = rand::rng();
    let mut casts: Vec<(u32, Option<usize>)> = Vec::new();
    for (s, en) in active(item) {
        let Some(def) = data.spells.enchantments.get(&en.id) else {
            continue;
        };
        for (kind, amount, _, spell) in def.effects {
            if kind != TYPE_COMBAT_SPELL || spell == 0 || !wanted(spell) {
                continue;
            }
            // vmangos: procs a minute if the spell has a rate, else the
            // enchantment's chance, else the weapon's speed's.
            let chance = match data.spells.enchant_ppm.get(&spell) {
                Some(ppm) if *ppm > 0.0 => ppm_chance(*ppm, delay),
                _ if amount > 0 => amount as f32,
                _ => delay as f32 * 1.8 / 1000.0,
            };
            if rng.random_range(0.0f32..100.0) < chance {
                casts.push((spell, (s == TEMP_SLOT && en.charges > 0).then_some(s)));
            }
        }
    }
    if let Some(t) = data.item(item.entry) {
        for s in t
            .spells
            .iter()
            .filter(|s| s.id != 0 && s.trigger == 2 && wanted(s.id))
        {
            let chance = if s.ppm_rate > 0.0 {
                ppm_chance(s.ppm_rate, delay)
            } else {
                match data.spells.get(s.id) {
                    Some(x) if x.proc_chance > 100 => ppm_chance(1.0, delay),
                    Some(x) => x.proc_chance as f32,
                    None => 0.0,
                }
            };
            if chance >= 100.0 || rng.random_range(0.0f32..100.0) < chance {
                casts.push((s.id, None));
            }
        }
    }
    for (spell, charge) in casts {
        if !map.unit_view(e.target).is_some_and(|u| u.alive) {
            break;
        }
        let helpful = data
            .spells
            .get(spell)
            .is_some_and(crate::game::map::auras::positive);
        let target = if helpful { e.actor } else { e.target };
        crate::game::map::spells::cast_triggered(map, e.actor, spell, Some(target));
        if let Some(slot) = charge {
            spend_charge(map, e.actor, item_guid, slot);
        }
    }
}

fn spend_charge(map: &mut Map, player: Guid, item: Guid, slot: usize) {
    let left = {
        let Some(p) = map.players.get_mut(&player) else {
            return;
        };
        let Some(i) = p.items.iter_mut().find(|x| x.guid == item) else {
            return;
        };
        let mut e = get(i, slot);
        e.charges = e.charges.saturating_sub(1);
        set(i, slot, e);
        e.charges
    };
    if left == 0 {
        clear(map, player, item, slot);
    }
}
