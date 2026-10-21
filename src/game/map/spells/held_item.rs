//! Effect 92, `ENCHANT_HELD_ITEM`: the weapon totems' enchantment
//! (Windfury Totem's, Flametongue Totem's), Feedback's and the
//! Firestones' on the main hand of each it lands on, as mangos-classic's
//! `Spell::EffectEnchantHeldItem`; and what Improved Weapon Totems makes of
//! the totems' (the item's enchantment modifier).

use wow_world_messages::Guid;

use crate::game::data::spells::SpellTemplate;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::enchant::{self, TEMP_SLOT};
use crate::game::map::inventory::SLOT_MAIN_HAND;
use crate::game::store::BAG_PLAYER;

pub const EFFECT_ENCHANT_HELD_ITEM: u32 = 92;
/// Improved Weapon Totems, its two ranks: its first effect for Windfury
/// Totem's, its second for Flametongue Totem's.
const IMPROVED_WEAPON_TOTEMS: [u32; 2] = [29192, 29193];
/// The weapon totems' effects' family flag (mangos's `0x04000000`).
const WEAPON_TOTEM_FLAG: u64 = 0x0400_0000;
/// Windfury Totem's icon (mangos's `SpellIconID == 1397`).
const WINDFURY_ICON: u32 = 1397;
/// Flametongue Totem's procs' family flag, and the attack they cast
/// (mangos's `EffectDummy`, mask `0x400000000`, casting 16368).
const FLAMETONGUE_TOTEM_PROC_FLAG: u64 = 0x4_0000_0000;
const FLAMETONGUE_TOTEM_ATTACK: u32 = 16368;
const FAMILY_SHAMAN: u32 = 11;

/// Effect `index` of `s` on `target`: the enchantment its misc value names
/// in the temporary slot of the main hand a player wields, for the spell's
/// duration, else its points in seconds, else ten seconds; not over a
/// different temporary one (mangos's `EffectEnchantHeldItem`). A weapon
/// totem's caster's owner with Improved Weapon Totems makes it stronger
/// by the talent's percent.
pub fn enchant(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    amount: i32,
) {
    let id = s.effects[index].misc_value.max(0) as u32;
    if id == 0 || !guid::is_player(target) || !map.realm.data.spells.enchantments.contains_key(&id)
    {
        return;
    }
    let Some(item) = map.players.get(&target).and_then(|p| {
        p.item_at(BAG_PLAYER, SLOT_MAIN_HAND)
            .map(|i| &p.items[i])
            .map(|i| (i.guid, enchant::get(i, TEMP_SLOT).id))
    }) else {
        return;
    };
    let (item, present) = item;
    if present != 0 && present != id {
        return;
    }
    let level = map
        .unit_fields(caster)
        .map_or(1, |f| f.get_u32(crate::game::fields::UNIT_FIELD_LEVEL));
    let secs_ms = map
        .realm
        .data
        .spells
        .duration(s, level)
        .filter(|ms| *ms > 0)
        .or_else(|| (amount > 0).then(|| amount as u32 * 1000))
        .unwrap_or(10_000);
    enchant::enchant_item(map, target, item, id, Some((secs_ms, 0)));
    let modifier = (s.family_name == FAMILY_SHAMAN && s.family_flags & WEAPON_TOTEM_FLAG != 0)
        .then(|| totem_modifier(map, caster, s))
        .flatten();
    match modifier {
        Some(m) => {
            map.spells.enchant_modifiers.insert(item, (id, m));
        }
        None => {
            map.spells.enchant_modifiers.remove(&item);
        }
    }
}

/// What the totem's owner's Improved Weapon Totems gives its enchantment:
/// its first effect's percent for Windfury Totem, its second's for
/// Flametongue Totem.
fn totem_modifier(map: &Map, caster: Guid, s: &SpellTemplate) -> Option<i32> {
    let owner = map.creature(caster).and_then(|c| c.owner).unwrap_or(caster);
    let effect = if s.spell_icon == WINDFURY_ICON { 0 } else { 1 };
    let a = crate::game::map::auras::auras(map, owner)?;
    IMPROVED_WEAPON_TOTEMS.iter().find_map(|talent| {
        a.holders
            .iter()
            .find(|h| h.spell == *talent)
            .and_then(|h| h.effects.iter().find(|x| x.effect as usize == effect))
            .map(|x| x.amount)
    })
}

/// The modifier of `item`'s temporary enchantment, while it is the one
/// the modifier was given with.
pub fn modifier(map: &Map, player: Guid, item: Guid) -> i32 {
    let Some((id, m)) = map.spells.enchant_modifiers.get(&item).copied() else {
        return 0;
    };
    let current = map
        .players
        .get(&player)
        .and_then(|p| p.items.iter().find(|i| i.guid == item))
        .map(|i| enchant::get(i, TEMP_SLOT).id);
    if current == Some(id) { m } else { 0 }
}

/// A weapon enchantment's spell, cast from `item`, with what its modifier
/// makes of it: Windfury Totem's attack (aura 99) its attack power raised
/// by the modifier's percent (mangos's `WindfuryTotemAura`). `None` for a
/// spell the modifier doesn't change.
pub fn modified(map: &Map, player: Guid, item: Guid, s: &SpellTemplate) -> Option<SpellTemplate> {
    let m = modifier(map, player, item);
    if m == 0 || s.family_name != FAMILY_SHAMAN || s.spell_icon != WINDFURY_ICON {
        return None;
    }
    let level = map
        .unit_fields(player)
        .map_or(1, |f| f.get_u32(crate::game::fields::UNIT_FIELD_LEVEL));
    let mut out = s.clone();
    for i in 0..3 {
        let e = &s.effects[i];
        if e.effect == 6 && e.aura == crate::game::map::auras::AURA_MOD_ATTACK_POWER {
            let v = super::points(s, i, level);
            let v = v + v * m / 100;
            let e = &mut out.effects[i];
            e.base_points = v - 1;
            e.base_dice = 1;
            e.die_sides = 0;
            e.dice_per_level = 0.0;
            e.real_points_per_level = 0.0;
        }
    }
    Some(out)
}

/// Whether `s` is a Flametongue Totem's proc, and if so it done: the
/// totem's attack for its points a hundredth a second of the speed of the
/// weapon it came from, raised by the weapon's modifier, with no spell
/// power (mangos's `EffectDummy`, mask `0x400000000`).
pub fn flametongue_totem(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    amount: i32,
    item: Option<Guid>,
) -> bool {
    if s.family_name != FAMILY_SHAMAN || s.family_flags & FLAMETONGUE_TOTEM_PROC_FLAG == 0 {
        return false;
    }
    let data = map.realm.data.clone();
    let Some(item) = item else {
        return true;
    };
    let Some(entry) = map
        .players
        .get(&caster)
        .and_then(|p| p.items.iter().find(|i| i.guid == item))
        .map(|i| i.entry)
    else {
        return true;
    };
    let speed = data.item(entry).map_or(2000, |t| t.delay.max(1));
    let m = modifier(map, caster, item);
    let damage = (amount as f32 * 0.01 * speed as f32 / 1000.0 * (m as f32 + 100.0) / 100.0) as i32;
    let Some(attack) = data.spells.get(FLAMETONGUE_TOTEM_ATTACK) else {
        return true;
    };
    let mut strike = attack.clone();
    strike.attributes[3] |= crate::game::data::spells::ATTR_EX3_IGNORE_CASTER_MODIFIERS;
    super::scripts::cast_template_with_points(
        map,
        caster,
        strike,
        target,
        [Some(damage.max(1)), None, None],
    );
    true
}
