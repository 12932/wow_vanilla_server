//! Spells on the trade window's seventh slot, as Thistle Tea's
//! `Trade.Spells` has them: a player enchants or unlocks the item their
//! partner shows in the slot not traded, which stays its owner's. The cast
//! waits in the caster's offer, shown in both windows, and goes off when
//! both accept: checked again, the caster paying its reagents, power and a
//! cast item's charge, the item changed, the caster's skill raised.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::items::Body;
use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::death::Death;
use crate::game::map::professions::{
    EFFECT_ENCHANT_ITEM, EFFECT_ENCHANT_ITEM_TEMPORARY, EFFECT_OPEN_LOCK, EFFECT_OPEN_LOCK_ITEM,
    ITEM_FLAG_UNLOCKED,
};
use crate::game::map::spells::checks::fail;
use crate::game::map::trade::NOT_TRADED;

/// `SpellCastResult`s of a trade's spell.
pub const ALREADY_OPEN: u8 = 0x06;
pub const ITEM_NOT_READY: u8 = 0x28;
pub const NOT_TRADEABLE: u8 = 0x3f;
pub const NOT_TRADING: u8 = 0x40;
/// `AttributesEx2`: it enchants only the caster's own items.
const ATTR_EX2_ENCHANT_OWN_ITEM_ONLY: u32 = 0x2000;
/// `SpellItemEnchantment` flags: it binds what it is on.
const ENCHANTMENT_CAN_SOULBOUND: u32 = 0x1;

/// A spell waiting in a trade, on the partner's item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeCast {
    pub spell: u32,
    /// The partner's item, in the slot not traded.
    pub target: Guid,
    /// The caster's item it is cast from: a key, an oil.
    pub item: Option<Guid>,
}

/// The trade slot a cast's targets name, if they name one.
fn trade_slot(t: &SpellCastTargets) -> Option<u64> {
    match t.target_flags.get_item()? {
        SpellCastTargets_SpellCastTargetFlags_Item::TradeItem { trade_item } => {
            Some(trade_item.guid())
        }
        SpellCastTargets_SpellCastTargetFlags_Item::Item { .. } => None,
    }
}

/// `CMSG_CAST_SPELL` and `CMSG_USE_ITEM` at a trade slot.
pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    match &msg {
        ClientOpcodeMessage::CMSG_CAST_SPELL(m) => {
            let Some(slot) = trade_slot(&m.targets) else {
                return Some(msg);
            };
            cast(map, g, m.spell, slot, None);
            None
        }
        ClientOpcodeMessage::CMSG_USE_ITEM(m) => {
            let Some(slot) = trade_slot(&m.targets) else {
                return Some(msg);
            };
            use_item(map, g, m.bag_index, m.bag_slot, slot);
            None
        }
        _ => Some(msg),
    }
}

fn result(map: &Map, g: Guid, spell: u32, code: Option<u8>) {
    if let Some(p) = map.players.get(&g) {
        let mut b = Body::default();
        b.u32(spell);
        match code {
            Some(code) => b.u8(2).u8(code),
            None => b.u8(0),
        };
        p.outbound
            .send_frame(b.frame(crate::game::map::spells::SMSG_CAST_RESULT));
    }
}

/// The partner of `g`'s open trade.
fn partner(map: &Map, g: Guid) -> Option<Guid> {
    let t = map.players.get(&g)?.trade.as_ref()?;
    t.open.then_some(t.with)
}

fn shown(map: &Map, owner: Guid) -> Option<Guid> {
    map.players.get(&owner)?.trade.as_ref()?.items[NOT_TRADED]
}

/// An item's on-use spell cast at a trade slot.
fn use_item(map: &mut Map, g: Guid, bag: u8, slot: u8, trade_slot: u64) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let Some(i) = p.item_at(bag, slot) else {
        return;
    };
    let item = p.items[i].guid;
    let Some((index, spell)) = data.item(p.items[i].entry).and_then(|t| {
        t.spells
            .iter()
            .enumerate()
            .find(|(_, s)| s.id != 0 && s.trigger == 0)
            .map(|(k, s)| (k, s.id))
    }) else {
        return;
    };
    if let Some(code) = crate::game::map::items::check_use(map, g, item, index) {
        result(map, g, spell, Some(code));
        return;
    }
    cast(map, g, spell, trade_slot, Some(item));
}

/// `g` casts `spell` at trade slot `slot`: on the partner's item in the
/// slot not traded, it waits in `g`'s offer.
pub fn cast(map: &mut Map, g: Guid, spell: u32, slot: u64, item: Option<Guid>) {
    if slot != NOT_TRADED as u64 {
        result(map, g, spell, Some(ITEM_NOT_READY));
        return;
    }
    let Some(with) = partner(map, g) else {
        result(map, g, spell, Some(NOT_TRADING));
        return;
    };
    let Some(target) = shown(map, with) else {
        result(map, g, spell, Some(fail::ITEM_GONE));
        return;
    };
    let c = TradeCast {
        spell,
        target,
        item,
    };
    if let Some(code) = check(map, g, with, &c) {
        result(map, g, spell, Some(code));
        return;
    }
    if let Some(t) = map.players.get_mut(&g).and_then(|p| p.trade.as_mut()) {
        t.spell = Some(c);
    }
    // Not cast now: the client's cast ends quietly.
    result(map, g, spell, Some(fail::DONT_REPORT));
    crate::game::map::trade::changed(map, g);
    crate::game::map::trade::send_offer(map, g, g);
}

fn is_enchant(effect: u32) -> bool {
    matches!(effect, EFFECT_ENCHANT_ITEM | EFFECT_ENCHANT_ITEM_TEMPORARY)
}

fn is_lock(effect: u32) -> bool {
    matches!(effect, EFFECT_OPEN_LOCK | EFFECT_OPEN_LOCK_ITEM)
}

/// Why `caster` can't cast `c` on `owner`'s item.
pub fn check(map: &Map, caster: Guid, owner: Guid, c: &TradeCast) -> Option<u8> {
    let data = &map.realm.data;
    let Some(s) = data.spells.get(c.spell) else {
        return Some(fail::NOT_KNOWN);
    };
    let (Some(p), Some(o)) = (map.players.get(&caster), map.players.get(&owner)) else {
        return Some(NOT_TRADING);
    };
    if p.death != Death::Alive {
        return Some(fail::CASTER_DEAD);
    }
    if p.caster.casting.is_some() {
        return Some(fail::SPELL_IN_PROGRESS);
    }
    let target = o
        .trade
        .as_ref()
        .and_then(|t| t.items[NOT_TRADED])
        .filter(|t| *t == c.target)
        .and_then(|t| o.item_by_guid(t))
        .map(|i| &o.items[i]);
    let Some(target) = target else {
        return Some(fail::ITEM_GONE);
    };
    let enchant = s.effects.iter().any(|e| is_enchant(e.effect));
    let lock = s.effects.iter().any(|e| is_lock(e.effect));
    if !enchant && !lock {
        return Some(fail::BAD_TARGETS);
    }
    if s.attr(2, ATTR_EX2_ENCHANT_OWN_ITEM_ONLY) {
        return Some(NOT_TRADEABLE);
    }
    if c.item.is_none() && !p.spells.contains(&s.id) {
        return Some(fail::NOT_KNOWN);
    }
    let tradeable = s.effects.iter().filter(|e| is_enchant(e.effect)).all(|e| {
        let id = e.misc_value.max(0) as u32;
        data.spells.enchantments.contains_key(&id)
            && data.crafts.enchant_flags.get(&id).copied().unwrap_or(0) & ENCHANTMENT_CAN_SOULBOUND
                == 0
    });
    if !tradeable {
        return Some(NOT_TRADEABLE);
    }
    let now = map.now;
    if p.caster.cooldowns.get(&s.id).is_some_and(|t| *t > now)
        || (s.category != 0
            && p.caster
                .category_cooldowns
                .get(&s.category)
                .is_some_and(|t| *t > now))
    {
        return Some(fail::NOT_READY);
    }
    if c.item.is_none() {
        let index = s.power_type.min(4) as u16;
        let cost = crate::game::map::spells::power_cost(map, caster, s);
        if cost > p.fields.get_u32(UNIT_FIELD_POWER1 + index) {
            return Some(fail::NO_POWER);
        }
    }
    // What the caster offers isn't theirs to spend.
    let offered: Vec<Guid> = p
        .trade
        .as_ref()
        .map(|t| t.items[..NOT_TRADED].iter().flatten().copied().collect())
        .unwrap_or_default();
    let spare = |entry: u32| -> u32 {
        p.items
            .iter()
            .filter(|i| i.entry == entry && !offered.contains(&i.guid))
            .map(|i| i.count())
            .sum()
    };
    if !p.gm && s.reagents.iter().any(|(r, n)| *r != 0 && spare(*r) < *n) {
        return Some(fail::REAGENTS);
    }
    if s.totems.iter().any(|t| *t != 0 && p.count_of(*t) == 0) {
        return Some(fail::TOTEMS);
    }
    let Some(t) = data.item(target.entry) else {
        return Some(fail::BAD_TARGETS);
    };
    if enchant {
        let fits = (s.equipped_item_class < 0 || t.class as i32 == s.equipped_item_class)
            && (s.equipped_item_subclass_mask <= 0
                || s.equipped_item_subclass_mask & (1 << t.subclass) != 0)
            && (s.equipped_item_inventory_type_mask <= 0
                || s.equipped_item_inventory_type_mask & (1 << t.inventory_type) != 0);
        if !fits || target.count() != 1 {
            return Some(fail::BAD_TARGETS);
        }
        // A lasting enchantment wants an item of its level (mangos's
        // `CheckItems`).
        if s.has_effect(EFFECT_ENCHANT_ITEM) && t.item_level < s.base_level {
            return Some(fail::LOWLEVEL);
        }
    }
    if lock {
        if t.lock_id == 0 || target.fields.get_u32(ITEM_FIELD_FLAGS) & ITEM_FLAG_UNLOCKED != 0 {
            return Some(ALREADY_OPEN);
        }
        for (index, _) in s
            .effects
            .iter()
            .enumerate()
            .filter(|(_, e)| is_lock(e.effect))
        {
            let cast_item = c
                .item
                .and_then(|g| p.item_by_guid(g))
                .map(|i| p.items[i].entry);
            if let Err(code) =
                crate::game::map::professions::resolve_lock(data, p, s, index, t.lock_id, cast_item)
            {
                return Some(code);
            }
        }
    }
    if let Some(item) = c.item {
        let index = p
            .item_by_guid(item)
            .filter(|_| !offered.contains(&item))
            .and_then(|i| data.item(p.items[i].entry))
            .and_then(|t| t.spells.iter().position(|k| k.id == s.id && k.trigger == 0));
        let Some(index) = index else {
            return Some(fail::ITEM_GONE);
        };
        // An item the caster may not use (a key of a skill they lack) isn't
        // theirs to cast with, as at any use of it (mangos's `CanUseItem`;
        // Thistle Tea's item gone).
        let usable = p
            .item_by_guid(item)
            .and_then(|i| data.item(p.items[i].entry))
            .is_some_and(|t| crate::game::map::inventory::can_use(p, data, t).is_ok());
        if !usable {
            return Some(fail::ITEM_GONE);
        }
        if let Some(code) = crate::game::map::items::check_use(map, caster, item, index) {
            return Some(code);
        }
    }
    None
}

/// Both accepted: each side's spell, checked again. A spell that fails
/// now goes from its offer, and both are back to trading; `None` then.
pub fn ready(map: &mut Map, a: Guid, b: Guid) -> Option<Vec<(Guid, Guid, TradeCast)>> {
    let mut casts = Vec::new();
    for (caster, owner) in [(a, b), (b, a)] {
        let Some(c) = map
            .players
            .get(&caster)
            .and_then(|p| p.trade.as_ref())
            .and_then(|t| t.spell)
        else {
            continue;
        };
        if let Some(code) = check(map, caster, owner, &c) {
            result(map, caster, c.spell, Some(code));
            for g in [a, b] {
                if let Some(t) = map.players.get_mut(&g).and_then(|p| p.trade.as_mut()) {
                    t.accepted = false;
                    if g == caster {
                        t.spell = None;
                    }
                }
                if let Some(p) = map.players.get(&g) {
                    p.outbound.send(&SMSG_TRADE_STATUS {
                        status: SMSG_TRADE_STATUS_TradeStatus::BackToTrade,
                    });
                }
            }
            crate::game::map::trade::send_offer(map, caster, owner);
            crate::game::map::trade::send_offer(map, caster, caster);
            return None;
        }
        casts.push((caster, owner, c));
    }
    Some(casts)
}

/// Spells that wait on an item no longer in the slot not traded go.
pub fn clear_stale(map: &mut Map, a: Guid, b: Guid) {
    for (caster, owner) in [(a, b), (b, a)] {
        let target = shown(map, owner);
        let stale = map
            .players
            .get_mut(&caster)
            .and_then(|p| p.trade.as_mut())
            .filter(|t| t.spell.is_some_and(|c| Some(c.target) != target))
            .map(|t| t.spell = None)
            .is_some();
        if stale {
            crate::game::map::trade::send_offer(map, caster, owner);
            crate::game::map::trade::send_offer(map, caster, caster);
        }
    }
}

/// The trade went through: each spell goes off on the item it waited on.
pub fn perform(map: &mut Map, casts: &[(Guid, Guid, TradeCast)]) {
    let data = map.realm.data.clone();
    for &(caster, owner, c) in casts {
        let Some(s) = data.spells.get(c.spell) else {
            continue;
        };
        pay(map, caster, s, &c);
        let mut opened = false;
        for index in 0..3 {
            let e = &s.effects[index];
            if is_enchant(e.effect) {
                let id = e.misc_value.max(0) as u32;
                let temporary = (e.effect == EFFECT_ENCHANT_ITEM_TEMPORARY).then(|| {
                    let secs = crate::game::map::spells::points(s, index, 1).max(1) as u32;
                    let charges = data.crafts.enchant_charges.get(&s.id).copied().unwrap_or(0);
                    (secs * 1000, charges)
                });
                crate::game::map::enchant::enchant_item(map, owner, c.target, id, temporary);
            } else if is_lock(e.effect) && !opened {
                opened = true;
                unlock(map, caster, owner, s, index, &c);
            }
        }
        if !opened && c.item.is_none() {
            crate::game::map::professions::recipe_skill_up(map, caster, s);
        }
        result(map, caster, s.id, None);
        let mut flags = SpellCastTargets_SpellCastTargetFlags::empty();
        flags = flags.set_item(SpellCastTargets_SpellCastTargetFlags_Item::Item { item: c.target });
        if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_GO {
            cast_item: c.item.unwrap_or(caster),
            caster,
            spell: s.id,
            flags: SMSG_SPELL_GO_CastFlags::empty(),
            hits: Vec::new(),
            misses: Vec::new(),
            targets: SpellCastTargets {
                target_flags: flags,
            },
        }) {
            map.send_to_observers(caster, &frame, true);
        }
    }
}

/// The caster pays: power (for a spell of their own), reagents, the
/// cooldown, and a cast item's charge.
fn pay(map: &mut Map, caster: Guid, s: &SpellTemplate, c: &TradeCast) {
    if c.item.is_none() {
        let m = crate::game::map::spells::mods::for_spell(map, caster, s);
        let cost = m.apply_ms(
            crate::game::map::spells::mods::OP_COST,
            crate::game::map::spells::power_cost(map, caster, s),
        );
        let index = s.power_type.min(4) as u16;
        if let Some(p) = map.players.get_mut(&caster) {
            let have = p.fields.get_u32(UNIT_FIELD_POWER1 + index);
            p.fields
                .set_u32(UNIT_FIELD_POWER1 + index, have.saturating_sub(cost));
        }
    }
    let gm = map.players.get(&caster).is_some_and(|p| p.gm);
    for (reagent, count) in s.reagents.iter().filter(|(r, _)| *r != 0 && !gm) {
        let mut left = *count;
        while left > 0 {
            let Some(i) = map
                .players
                .get(&caster)
                .and_then(|p| p.items.iter().position(|it| it.entry == *reagent))
            else {
                break;
            };
            let n = map.players[&caster].items[i].count().min(left);
            crate::game::map::inventory::take_item(map, caster, i, n);
            left -= n;
        }
    }
    let m = crate::game::map::spells::mods::for_spell(map, caster, s);
    crate::game::map::spells::cooldowns::start(map, caster, s, &m, c.item);
    if let Some(item) = c.item {
        crate::game::map::items::used(map, caster, item, s.id);
    }
}

/// A lock opened on the partner's item: it can be opened by its owner now,
/// and the caster's skill may rise (not with a key).
fn unlock(
    map: &mut Map,
    caster: Guid,
    owner: Guid,
    s: &SpellTemplate,
    index: usize,
    c: &TradeCast,
) {
    let data = map.realm.data.clone();
    let lock = map
        .players
        .get(&owner)
        .and_then(|o| o.item_by_guid(c.target).map(|i| o.items[i].entry))
        .and_then(|entry| data.item(entry))
        .map_or(0, |t| t.lock_id);
    let resolved = map.players.get(&caster).and_then(|p| {
        let cast_item = c
            .item
            .and_then(|g| p.item_by_guid(g))
            .map(|i| p.items[i].entry);
        crate::game::map::professions::resolve_lock(&data, p, s, index, lock, cast_item).ok()
    });
    if let Some(o) = map.players.get_mut(&owner)
        && let Some(i) = o.item_by_guid(c.target)
    {
        let flags = o.items[i].fields.get_u32(ITEM_FIELD_FLAGS);
        o.items[i]
            .fields
            .set_u32(ITEM_FIELD_FLAGS, flags | ITEM_FLAG_UNLOCKED);
    }
    if let Some(crate::game::map::professions::Opening {
        skill: Some(skill),
        required,
        gain: true,
        ..
    }) = resolved
    {
        let learned = map
            .players
            .get(&caster)
            .and_then(|p| crate::game::map::skills::value(p, skill))
            .unwrap_or(0);
        let chance = crate::game::map::professions::gather_gain_chance(skill, learned, required, 1);
        let by = map.cfg.rates.skill_gain_gathering;
        crate::game::map::professions::try_raise_by(map, caster, skill, chance, by);
    }
}

/// What a trade's window shows of a spell: the spell `g` cast on their
/// partner's item.
pub fn shown_spell(map: &Map, g: Guid) -> u32 {
    map.players
        .get(&g)
        .and_then(|p| p.trade.as_ref())
        .and_then(|t| t.spell)
        .map_or(0, |c| c.spell)
}
