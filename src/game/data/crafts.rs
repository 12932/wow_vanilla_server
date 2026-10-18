//! What professions and items need beyond the templates: locks, item sets,
//! random properties, enchantments' charges and fishing's skill by zone.

use ahash::AHashMap;
use rusqlite::Connection;

use super::load::rows;

/// `Lock` key types.
pub const LOCK_KEY_ITEM: u8 = 1;
pub const LOCK_KEY_SKILL: u8 = 2;

/// One way to open a lock: an item, or a lock type with the skill it
/// needs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct LockKey {
    pub kind: u8,
    /// The item for an item key; the lock type for a skill key.
    pub index: u32,
    pub skill: u32,
}

/// `ItemSet`: the set's items, and the spells it gives at each count.
#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct ItemSet {
    pub id: u32,
    pub items: Vec<u32>,
    /// (spell, items worn).
    pub spells: Vec<(u32, u32)>,
    /// A skill its bonuses ask for, and how much of it.
    pub required_skill: u32,
    pub required_skill_rank: u32,
}

#[derive(Debug, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct CraftData {
    /// `Lock`: its keys, in order.
    pub locks: AHashMap<u32, Vec<LockKey>>,
    pub item_sets: AHashMap<u32, ItemSet>,
    /// `ItemRandomProperties`: a property's enchantments.
    pub random_properties: AHashMap<u32, [u32; 5]>,
    /// A random property's suffix: "of the Bear".
    pub random_property_names: AHashMap<u32, String>,
    /// `item_enchantment_template`: an item's possible random properties
    /// and their chances.
    pub random_pools: AHashMap<u32, Vec<(u32, f32)>>,
    /// `spell_enchant_charges`: the charges an enchanting spell gives.
    pub enchant_charges: AHashMap<u32, u32>,
    /// `skill_fishing_base_level`: the skill a zone or area's fish need.
    pub fishing_skill: AHashMap<u32, i32>,
    /// `SpellItemEnchantment` flags, where not 0 (1: it binds the item).
    pub enchant_flags: AHashMap<u32, u32>,
    /// `item_required_target`: the creatures an item is used on, each
    /// alive (`true`) or dead.
    pub item_targets: AHashMap<u32, Vec<(u32, bool)>>,
}

pub fn load(db: &Connection, dbc: Option<&Connection>, d: &mut CraftData) {
    for (entry, ench, chance) in rows(
        db,
        "SELECT * FROM item_enchantment_template WHERE patch_min <= 10 AND patch_max >= 10",
        |c| (c.u32("entry"), c.u32("ench"), c.f32("chance")),
    ) {
        d.random_pools
            .entry(entry)
            .or_default()
            .push((ench, chance));
    }
    for (spell, charges) in rows(db, "SELECT * FROM spell_enchant_charges", |c| {
        (c.u32("entry"), c.u32("charges"))
    }) {
        d.enchant_charges.insert(spell, charges);
    }
    for (zone, skill) in rows(db, "SELECT * FROM skill_fishing_base_level", |c| {
        (c.u32("entry"), c.i32("skill"))
    }) {
        d.fishing_skill.insert(zone, skill);
    }
    for (item, creature, kind) in rows(db, "SELECT * FROM item_required_target", |c| {
        (c.u32("entry"), c.u32("target_entry"), c.u32("type"))
    }) {
        if matches!(kind, 1 | 2) {
            let targets = d.item_targets.entry(item).or_default();
            if !targets.contains(&(creature, kind == 1)) {
                targets.push((creature, kind == 1));
            }
        }
    }
    let Some(dbc) = dbc else {
        return;
    };
    for (id, flags) in rows(dbc, "SELECT id, flags FROM SpellItemEnchantment", |c| {
        (c.u32("id"), c.u32("flags"))
    }) {
        if flags != 0 {
            d.enchant_flags.insert(id, flags);
        }
    }
    for (id, keys) in rows(dbc, "SELECT * FROM Lock", |c| {
        let keys: Vec<LockKey> = (0..8)
            .map(|i| LockKey {
                kind: c.u8(&format!("ty_{i}")),
                index: c.u32(&format!("property_{i}")),
                skill: c.u32(&format!("required_skill_{i}")),
            })
            .filter(|k| k.kind != 0)
            .collect();
        (c.u32("id"), keys)
    }) {
        d.locks.insert(id, keys);
    }
    for s in rows(dbc, "SELECT * FROM ItemSet", |c| ItemSet {
        id: c.u32("id"),
        items: (0..10)
            .map(|i| c.u32(&format!("items_{i}")))
            .filter(|i| *i != 0)
            .collect(),
        spells: (0..8)
            .map(|i| {
                (
                    c.u32(&format!("set_spell_{i}")),
                    c.u32(&format!("set_threshold_{i}")),
                )
            })
            .filter(|(s, _)| *s != 0)
            .collect(),
        required_skill: c.u32("required_skill"),
        required_skill_rank: c.u32("required_skill_rank"),
    }) {
        d.item_sets.insert(s.id, s);
    }
    for (id, enchants, suffix) in rows(dbc, "SELECT * FROM ItemRandomProperties", |c| {
        (
            c.u32("id"),
            std::array::from_fn(|i| c.u32(&format!("spell_item_enchantment_{i}"))),
            c.string("suffix_en_gb"),
        )
    }) {
        d.random_properties.insert(id, enchants);
        if !suffix.is_empty() {
            d.random_property_names.insert(id, suffix);
        }
    }
}
