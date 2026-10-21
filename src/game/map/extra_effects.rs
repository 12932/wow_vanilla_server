//! Spell effects the spells module had no arm for, as Thistle Tea runs
//! them, behind the one hook [`effect`]: inebriation (100), reputation
//! (103), a taxi path flown (123), a pull towards the caster (124) and an
//! item changed into another (34); and the item and mount scripts. With
//! them, the effect immunity aura (37): a helpful immunity stops the harmful
//! effects of its kind, a harmful one the helpful.

use wow_world_messages::Guid;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_EFFECT_IMMUNITY};

pub const EFFECT_SUMMON_CHANGE_ITEM: u32 = 34;
pub const EFFECT_INEBRIATE: u32 = 100;
pub const EFFECT_REPUTATION: u32 = 103;
pub const EFFECT_SEND_TAXI: u32 = 123;
pub const EFFECT_PLAYER_PULL: u32 = 124;

const GRAVITY: f32 = 19.291_105;
const SMSG_MOVE_KNOCK_BACK: u16 = 0x0EF;

/// Effect `index` of `s` from `caster` landing on `target`, for `amount`:
/// whether it was one of these.
pub fn effect(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    amount: i32,
    item: Option<Guid>,
) -> bool {
    if crate::game::map::item_scripts::effect(map, caster, target, s, index, item) {
        return true;
    }
    let e = s.effects[index];
    match e.effect {
        EFFECT_INEBRIATE => {
            crate::game::map::intoxication::drink(map, target, amount);
        }
        EFFECT_REPUTATION if e.misc_value > 0 && guid::is_player(target) => {
            crate::game::map::reputation::spell_reward(map, target, e.misc_value as u32, amount);
        }
        EFFECT_SEND_TAXI if e.misc_value > 0 && guid::is_player(target) => {
            crate::game::map::npc::fly_path(map, target, e.misc_value as u32);
        }
        EFFECT_PLAYER_PULL => pull(map, caster, target, (e.misc_value.max(1) as f32) / 10.0),
        EFFECT_SUMMON_CHANGE_ITEM => {
            if target == caster
                && e.misc_value > 0
                && let Some(item) = item
            {
                change_item(map, caster, item, e.misc_value as u32);
            }
        }
        _ => return false,
    }
    true
}

/// Whether `target` is immune to effect `index` of `s`: an effect
/// immunity of its kind, of the other side, or of either if its spell
/// says so (`SPELL_ATTR_EX_IMMUNITY_TO_HOSTILE_AND_FRIENDLY_EFFECTS`: a
/// flag carrier isn't summoned), as Thistle Tea's `EffectImmunity` has it
/// (mangos's `IsImmuneToSpellEffect` asks no side); none for a spell that
/// ignores immunities.
pub fn effect_immune(map: &Map, target: Guid, s: &SpellTemplate, index: usize) -> bool {
    let kind = s.effects[index].effect as i32;
    if kind == 0 || s.attr(0, crate::game::data::spells::ATTR_NO_IMMUNITIES) {
        return false;
    }
    let harmful = !auras::positive(s);
    let data = &map.realm.data;
    auras::auras(map, target).is_some_and(|a| {
        a.holders.iter().any(|h| {
            h.effects
                .iter()
                .any(|x| x.kind == AURA_EFFECT_IMMUNITY && x.misc == kind)
                && (h.positive == harmful
                    || data
                        .spells
                        .get(h.spell)
                        .is_some_and(|i| i.attr(1, IMMUNITY_TO_HOSTILE_AND_FRIENDLY)))
        })
    })
}

/// `SPELL_ATTR_EX_IMMUNITY_TO_HOSTILE_AND_FRIENDLY_EFFECTS`.
const IMMUNITY_TO_HOSTILE_AND_FRIENDLY: u32 = 0x1_0000;

/// A pull: the target, a player, is thrown at the caster at `speed`, to
/// land at their feet.
fn pull(map: &mut Map, caster: Guid, target: Guid, speed: f32) {
    if caster == target || speed <= 0.0 {
        return;
    }
    let (Some(me), Some(them)) = (map.unit_view(caster), map.unit_view(target)) else {
        return;
    };
    let Some(p) = map.players.get(&target) else {
        return;
    };
    // As a knockback: not the dead, nor one rooted or stunned, on a
    // flight, or on a move the server drives (Thistle Tea's `pull`
    // shares the launch's restrictions).
    if !them.alive
        || p.flight.is_some()
        || p.driven.is_some()
        || p.charge.is_some()
        || p.auras.has(auras::AURA_MOD_ROOT)
        || p.auras.has(auras::AURA_MOD_STUN)
    {
        return;
    }
    let radius = |g: Guid| {
        map.unit_fields(g)
            .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
    };
    let reach = radius(caster) + radius(target);
    let (dx, dy) = (
        me.position.x - them.position.x,
        me.position.y - them.position.y,
    );
    let distance = (dx * dx + dy * dy).sqrt() - reach;
    if distance < 0.1 {
        return;
    }
    let flight = distance / speed;
    let vertical = (me.position.z - them.position.z) / flight + GRAVITY * flight / 2.0;
    let angle = dy.atan2(dx);
    crate::game::map::spells::interrupt(map, target);
    crate::game::map::ranged::cancel(map, target, true);
    let Some(p) = map.players.get_mut(&target) else {
        return;
    };
    // The flight begins a fall of its own.
    p.fall_from = None;
    let mut b = crate::game::data::items::Body::default();
    crate::game::update::write_packed_guid(&mut b.0, target);
    let impulse = [angle.cos(), angle.sin(), speed, -vertical];
    b.u32(p.outbound.order(crate::game::conn::Order::KnockBack {
        guid: target,
        impulse,
    }))
    .f32(impulse[0])
    .f32(impulse[1])
    .f32(impulse[2])
    .f32(impulse[3]);
    p.outbound.send_frame(b.frame(SMSG_MOVE_KNOCK_BACK));
}

/// The item cast from turns into `entry` where it lies, if the new item may
/// lie there (mangos's `ConvertItem`, Thistle Tea's replacements): in a bag
/// it goes in, worn where it may be worn (a two-hander putting the off
/// hand's item away, if there is room), a bag only if empty, and a unique
/// one only if no other is had, the bank's counted. It is a new item of its
/// own: its own random property and time, the old one's permanent and
/// temporary enchantments, and its wear in proportion. Refused, the old
/// item stays and the player is told why.
pub fn change_item(map: &mut Map, g: Guid, item: Guid, entry: u32) {
    use crate::game::map::enchant;
    use crate::game::map::inventory::{
        self, BAG_END, BAG_START, BANK_BAG_END, BANK_BAG_START, Fail, SLOT_MAIN_HAND, SLOT_OFF_HAND,
    };
    use crate::game::store::BAG_PLAYER;
    let data = map.realm.data.clone();
    let Some(new) = data.item(entry) else {
        return;
    };
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(i) = p.items.iter().position(|it| it.guid == item) else {
        return;
    };
    let old = &p.items[i];
    let at = (old.bag, old.slot);
    // A new item, of a GUID of its own (mangos's `ConvertItem`:
    // `Item::CreateItem`, the old one destroyed).
    let counter = map
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut replacement = crate::game::map::player::Item::new(
        &data,
        crate::game::guid::item(counter),
        g,
        entry,
        at.0,
        at.1,
        1,
    );
    let property = enchant::roll_random_property(&data, entry, &mut rand::rng());
    if property != 0 {
        enchant::set_random_property(&data, &mut replacement, property);
    }
    for slot in [enchant::PERM_SLOT, enchant::TEMP_SLOT] {
        let e = enchant::get(old, slot);
        if e.id != 0 {
            enchant::set(&mut replacement, slot, e);
        }
    }
    let (wear, max) = (
        old.fields.get_u32(ITEM_FIELD_DURABILITY),
        old.fields.get_u32(ITEM_FIELD_MAXDURABILITY),
    );
    if new.max_durability > 0 && max > 0 && wear < max {
        let loss = (new.max_durability * (max - wear) / max).max(1);
        replacement.fields.set_u32(
            ITEM_FIELD_DURABILITY,
            new.max_durability.saturating_sub(loss),
        );
    }
    let bag_slot = at.0 == BAG_PLAYER
        && ((BAG_START..BAG_END).contains(&at.1) || (BANK_BAG_START..BANK_BAG_END).contains(&at.1));
    let others = p
        .items
        .iter()
        .filter(|x| x.entry == entry && x.guid != item)
        .filter(|x| !crate::game::map::vendor::in_buyback(x.bag, x.slot))
        .count() as u32;
    let worn = at.0 == BAG_PLAYER && at.1 < inventory::EQUIPMENT_END;
    let why = if bag_slot && p.items.iter().any(|x| x.bag == at.1) {
        Some(Fail::OnlyEmptyBags)
    } else if new.max_count > 0 && others + 1 > new.max_count {
        Some(Fail::CantCarryMore)
    } else {
        inventory::may_place(p, &data, &replacement, at.0, at.1).err()
    };
    // A two-hander's off hand put away: where it can be carried.
    let off_hand = match p.item_at(BAG_PLAYER, SLOT_OFF_HAND) {
        Some(off)
            if why.is_none() && at == (BAG_PLAYER, SLOT_MAIN_HAND) && new.inventory_type == 17 =>
        {
            let t = data.item(p.items[off].entry);
            match crate::game::map::items::carry_slots(&data, p, t)
                .into_iter()
                .find(|s| p.item_at(s.0, s.1).is_none())
            {
                Some(place) => Some(Ok((off, place))),
                None => Some(Err(Fail::TwoHanded)),
            }
        }
        _ => None,
    };
    if let Some(f) = why.or(off_hand.and_then(|o| o.err())) {
        inventory::send_fail(p, f, item);
        return;
    }
    if let Some(Ok((off, (bag, slot)))) = off_hand {
        p.items[off].bag = bag;
        p.items[off].slot = slot;
    }
    if worn || bag_slot {
        crate::game::map::items::on_equip(&data, &mut replacement);
    }
    let old = p.items.remove(i);
    p.outbound
        .send(&wow_world_messages::vanilla::SMSG_DESTROY_OBJECT { guid: old.guid });
    let mut batch = crate::game::update::Batch::new();
    batch.push(&crate::game::update::create_block(
        &replacement.create(),
        Visibility::Owner,
    ));
    for frame in batch.finish() {
        p.outbound.send_frame(frame);
    }
    // A temporary enchantment kept, with its time and charges: its time
    // told under the new GUID.
    let kept = enchant::get(&replacement, enchant::TEMP_SLOT);
    if kept.id != 0 && kept.duration_ms > 0 {
        p.outbound.send(
            &wow_world_messages::vanilla::SMSG_ITEM_ENCHANT_TIME_UPDATE {
                item: replacement.guid,
                slot: enchant::TEMP_SLOT as u32,
                duration: kept.duration_ms / 1000,
                player: g,
            },
        );
    }
    p.items.push(replacement);
    if let Some(it) = p.items.last_mut() {
        it.fields.clear_dirty();
    }
    p.refresh_inventory(&data);
    p.gear_dirty = true;
    let old_entry = old.entry;
    crate::game::map::quests::items_changed(map, g, old_entry);
    crate::game::map::quests::items_added(map, g, entry, 1);
}
