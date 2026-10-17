//! Cooldowns: a spell's own and its category's, with talents' modifiers;
//! those an item's spell carries; the ones that start only when the spell's
//! aura goes (Stealth) or its object does (the Ritual of Doom's portal);
//! schools an interrupt locks; and keeping auras and cooldowns across a
//! logout.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemSpell;
use crate::game::data::spells::SpellTemplate;
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::player::Player;

use super::mods::{self, Mods};

/// `Attributes`: the cooldown starts when the aura goes.
const ATTR_COOLDOWN_ON_EVENT: u32 = 0x200_0000;

/// Start `s`'s cooldowns for `caster`: the item's, if an item cast it, or
/// the spell's; a "cooldown on event" spell's waits for its aura to go.
pub fn start(map: &mut Map, caster: Guid, s: &SpellTemplate, m: &Mods, item: Option<Guid>) {
    let now = map.now;
    let data = map.realm.data.clone();
    let item_spell = item.and_then(|g| {
        let p = map.players.get(&caster)?;
        let i = p.item_by_guid(g)?;
        let t = data.items.get(&p.items[i].entry)?;
        t.spells.iter().find(|x| x.id == s.id).copied()
    });
    let (recovery, category, category_recovery) = match item_spell {
        Some(x) if x.cooldown > 0 || x.category_cooldown > 0 => (
            x.cooldown.max(0) as u32,
            x.category,
            x.category_cooldown.max(0) as u32,
        ),
        _ => (s.recovery_time, s.category, s.category_recovery_time),
    };
    let recovery = m.apply_ms(mods::OP_COOLDOWN, recovery);
    let category_recovery = m.apply_ms(mods::OP_COOLDOWN, category_recovery);
    if recovery == 0 && category_recovery == 0 {
        return;
    }
    if s.attr(0, ATTR_COOLDOWN_ON_EVENT) {
        if let Some(c) = map.caster_mut(caster)
            && !c.pending_cooldowns.contains(&s.id)
        {
            c.pending_cooldowns.push(s.id);
        }
        return;
    }
    if let Some(c) = map.caster_mut(caster) {
        if recovery > 0 {
            c.cooldowns
                .insert(s.id, now + Duration::from_millis(u64::from(recovery)));
        }
        if category != 0 && category_recovery > 0 {
            c.category_cooldowns.insert(
                category,
                now + Duration::from_millis(u64::from(category_recovery)),
            );
        }
    }
    let ms = recovery.max(category_recovery);
    send(map, caster, &[(s.id, ms)]);
    if item.is_some()
        && let Some(p) = map.players.get(&caster)
    {
        p.outbound.send(&SMSG_ITEM_COOLDOWN {
            guid: item.unwrap_or_default(),
            id: s.id,
        });
    }
}

/// `SMSG_SPELL_COOLDOWN` to a player.
pub fn send(map: &Map, player: Guid, cooldowns: &[(u32, u32)]) {
    if let Some(p) = map.players.get(&player)
        && !cooldowns.is_empty()
    {
        p.outbound.send(&SMSG_SPELL_COOLDOWN {
            guid: player,
            cooldowns: cooldowns
                .iter()
                .map(|(id, ms)| SpellCooldownStatus {
                    id: *id,
                    cooldown_time: Duration::from_millis(u64::from(*ms)),
                })
                .collect(),
        });
    }
}

/// `SMSG_CLEAR_COOLDOWN`: a cooldown ended early.
pub fn send_clear(map: &Map, player: Guid, spell: u32) {
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_CLEAR_COOLDOWN {
            id: spell,
            target: player,
        });
    }
}

/// A holder came off: a cooldown that waited for it starts now.
pub fn aura_gone(map: &mut Map, target: Guid, spell: u32) {
    release(map, target, spell, true);
}

/// A cooldown that waited for an event is let go (Thistle Tea's
/// `ActivateCooldown`): it starts now, or, `start` false (a ritual that
/// was never done), there is none and the spell may be cast again.
pub fn release(map: &mut Map, target: Guid, spell: u32, start: bool) {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(c) = map.caster_mut(target) else {
        return;
    };
    let Some(i) = c.pending_cooldowns.iter().position(|s| *s == spell) else {
        return;
    };
    c.pending_cooldowns.remove(i);
    if !start {
        send_clear(map, target, spell);
        return;
    }
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    let recovery = s.recovery_time.max(s.category_recovery_time);
    if let Some(c) = map.caster_mut(target) {
        c.cooldowns
            .insert(spell, now + Duration::from_millis(u64::from(recovery)));
    }
    if let Some(p) = map.players.get(&target) {
        p.outbound.send(&SMSG_COOLDOWN_EVENT {
            id: spell,
            guid: target,
        });
    }
}

/// Whether an item's spell is still cooling down.
pub fn item_on_cooldown(map: &Map, player: Guid, spell: &ItemSpell) -> bool {
    let now = map.now;
    map.caster(player).is_some_and(|c| {
        c.cooldowns.get(&spell.id).is_some_and(|t| *t > now)
            || (spell.category != 0
                && c.category_cooldowns
                    .get(&spell.category)
                    .is_some_and(|t| *t > now))
    })
}

/// An interrupt locks `target` out of the interrupted spell's school for
/// `ms`; a player's client hears of each spell of that school it knows.
pub fn lock_school(map: &mut Map, target: Guid, interrupted: u32, ms: u32) {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(interrupted) else {
        return;
    };
    let mask = s.school_mask();
    if let Some(c) = map.caster_mut(target) {
        c.school_lockout
            .push((mask, now + Duration::from_millis(u64::from(ms))));
    }
    let Some(p) = map.players.get(&target) else {
        return;
    };
    let locked: Vec<(u32, u32)> = p
        .spells
        .iter()
        .filter_map(|id| data.spells.get(*id))
        .filter(|t| !t.passive() && t.school_mask() & mask != 0 && !t.melee_or_ranged())
        .map(|t| (t.id, ms))
        .collect();
    send(map, target, &locked);
}

/// An aura as it is kept across a logout.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedAura {
    pub spell: u32,
    pub caster: u64,
    pub item: u64,
    pub stacks: u32,
    pub charges: u32,
    /// Milliseconds left; `None` for one that lasts.
    pub left: Option<u32>,
    /// Each effect's index and amount.
    pub amounts: Vec<(u8, i32)>,
    pub combo: u8,
    /// From a triggered spell (it keeps its lesser worth).
    pub triggered: bool,
}

/// The auras a player keeps across a logout: not passive ones, not gear's,
/// not another's lasting party auras, not a channel's.
pub fn saved_auras(p: &Player, now: Instant) -> Vec<SavedAura> {
    let channel = p
        .caster
        .casting
        .as_ref()
        .filter(|c| c.channel_until.is_some())
        .map(|c| c.spell);
    p.auras
        .holders
        .iter()
        .filter(|h| !h.passive && !h.equipped)
        .filter(|h| !(h.caster != p.guid && h.expires.is_none()))
        .filter(|h| Some(h.spell) != channel)
        .filter(|h| h.expires.is_none_or(|t| t > now))
        .map(|h| SavedAura {
            spell: h.spell,
            caster: h.caster.guid(),
            item: h.item.map_or(0, |i| i.guid()),
            stacks: h.stacks,
            charges: h.charges,
            left: h
                .expires
                .map(|t| t.saturating_duration_since(now).as_millis() as u32),
            amounts: h.effects.iter().map(|x| (x.effect, x.amount)).collect(),
            combo: h.combo,
            triggered: h.triggered,
        })
        .collect()
}

/// The cooldowns a player keeps: (spell, milliseconds left).
pub fn saved_cooldowns(p: &Player, now: Instant) -> Vec<(u32, u32)> {
    p.caster
        .cooldowns
        .iter()
        .filter(|(_, t)| **t > now)
        .map(|(id, t)| (*id, t.saturating_duration_since(now).as_millis() as u32))
        .collect()
}

/// The category cooldowns a player keeps: (category, milliseconds left).
pub fn saved_category_cooldowns(p: &Player, now: Instant) -> Vec<(u32, u32)> {
    p.caster
        .category_cooldowns
        .iter()
        .filter(|(_, t)| **t > now)
        .map(|(id, t)| (*id, t.saturating_duration_since(now).as_millis() as u32))
        .collect()
}

/// A player's saved auras and cooldowns come back at login.
pub fn restore(map: &mut Map, player: Guid) {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let saved = std::mem::take(&mut p.row.state.auras);
    let mut cooldowns = std::mem::take(&mut p.row.state.cooldowns);
    let categories = std::mem::take(&mut p.row.state.category_cooldowns);
    for (id, ms) in &cooldowns {
        p.caster
            .cooldowns
            .insert(*id, now + Duration::from_millis(u64::from(*ms)));
    }
    for (category, ms) in &categories {
        p.caster
            .category_cooldowns
            .insert(*category, now + Duration::from_millis(u64::from(*ms)));
        // The client hears of each known spell in the category.
        for id in &p.spells {
            if data
                .spells
                .get(*id)
                .is_some_and(|s| s.category == *category)
                && !cooldowns.iter().any(|c| c.0 == *id)
            {
                cooldowns.push((*id, *ms));
            }
        }
    }
    for a in saved {
        let Some(s) = data.spells.get(a.spell) else {
            continue;
        };
        let effects: Vec<(usize, i32)> = a
            .amounts
            .iter()
            .filter(|(i, _)| (*i as usize) < 3)
            .map(|(i, v)| (*i as usize, *v))
            .collect();
        if effects.is_empty() {
            continue;
        }
        let caster = Guid::new(a.caster);
        let caster = if caster.is_zero() { player } else { caster };
        // Its ticks as its caster's modifiers have them now.
        let m = super::mods::for_spell(map, caster, s);
        let _ = auras::apply_spell(
            map,
            caster,
            player,
            s,
            &effects,
            auras::Context {
                item: (a.item != 0).then(|| Guid::new(a.item)),
                equipped: false,
                combo: a.combo,
                duration: Some(a.left),
                charges: Some(a.charges),
                triggered: a.triggered,
                linked: false,
                activation: (
                    m.flat[super::mods::OP_ACTIVATION_TIME],
                    m.pct[super::mods::OP_ACTIVATION_TIME],
                ),
            },
        );
        if let Some(h) = auras::auras_mut(map, player).and_then(|x| {
            x.holders
                .iter_mut()
                .find(|h| h.spell == a.spell && h.caster == caster)
        }) {
            h.stacks = a.stacks.max(1);
        }
    }
    send(map, player, &cooldowns);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_aura_round_trips_through_json() {
        let a = SavedAura {
            spell: 1243,
            caster: 7,
            stacks: 1,
            left: Some(1_799_000),
            amounts: vec![(0, 3)],
            triggered: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: SavedAura = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
        // One saved before the flag was kept reads as not triggered.
        let old: SavedAura = serde_json::from_str(r#"{"spell":1243,"caster":7}"#).unwrap();
        assert!(!old.triggered);
    }
}
