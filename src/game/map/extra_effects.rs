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
/// immunity of its kind, of the other polarity.
pub fn effect_immune(map: &Map, target: Guid, s: &SpellTemplate, index: usize) -> bool {
    let kind = s.effects[index].effect as i32;
    let harmful = !auras::positive(s);
    auras::auras(map, target).is_some_and(|a| {
        a.holders.iter().any(|h| {
            h.positive == harmful
                && h.effects
                    .iter()
                    .any(|x| x.kind == AURA_EFFECT_IMMUNITY && x.misc == kind)
        })
    })
}

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
    b.u32(0)
        .f32(angle.cos())
        .f32(angle.sin())
        .f32(speed)
        .f32(-vertical);
    p.outbound.send_frame(b.frame(SMSG_MOVE_KNOCK_BACK));
}

/// The item cast from turns into `entry` where it lies, its enchantments
/// kept and its wear in proportion.
pub fn change_item(map: &mut Map, g: Guid, item: Guid, entry: u32) {
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
    let old = p.items.remove(i);
    let mut row = old.to_row();
    let (wear, max) = (
        old.fields.get_u32(ITEM_FIELD_DURABILITY),
        old.fields.get_u32(ITEM_FIELD_MAXDURABILITY),
    );
    row.entry = entry;
    row.durability = if new.max_durability > 0 && max > 0 && wear < max {
        let loss = (new.max_durability * (max - wear) / max).max(1);
        new.max_durability.saturating_sub(loss)
    } else {
        new.max_durability
    };
    p.outbound
        .send(&wow_world_messages::vanilla::SMSG_DESTROY_OBJECT { guid: old.guid });
    let replacement = crate::game::map::player::Item::from_row(&data, &row, g);
    let mut batch = crate::game::update::Batch::new();
    batch.push(&crate::game::update::create_block(
        &replacement.create(),
        Visibility::Owner,
    ));
    for frame in batch.finish() {
        p.outbound.send_frame(frame);
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
