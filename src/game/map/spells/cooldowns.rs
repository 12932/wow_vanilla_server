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

/// A charmed creature's charm spell recovers over the time its slot
/// rolled, in place of its own (Thistle Tea's `CharmSpells`).
fn charm_cooldown(map: &Map, caster: Guid, spell: u32) -> Option<u32> {
    map.creature(caster)
        .and_then(|c| c.pet.as_ref())
        .and_then(|p| p.charm_cooldowns.iter().find(|(id, _)| *id == spell))
        .map(|(_, ms)| *ms)
}

/// Start `s`'s cooldowns for `caster`: the item's, if an item cast it, or
/// the spell's; a "cooldown on event" spell's waits for its aura to go.
pub fn start(map: &mut Map, caster: Guid, s: &SpellTemplate, m: &Mods, item: Option<Guid>) {
    let now = map.now;
    let data = map.realm.data.clone();
    let item_entry = item
        .and_then(|g| {
            let p = map.players.get(&caster)?;
            p.item_by_guid(g).map(|i| p.items[i].entry)
        })
        .unwrap_or(0);
    let item_spell = data
        .items
        .get(&item_entry)
        .and_then(|t| t.spells.iter().find(|x| x.id == s.id).copied());
    let (recovery, category, category_recovery) = match item_spell {
        Some(x) if x.cooldown > 0 || x.category_cooldown > 0 => (
            x.cooldown.max(0) as u32,
            x.category,
            x.category_cooldown.max(0) as u32,
        ),
        _ => (s.recovery_time, s.category, s.category_recovery_time),
    };
    let raw = charm_cooldown(map, caster, s.id).unwrap_or(recovery);
    let raw_category = category_recovery;
    let recovery = m.apply_ms(mods::OP_COOLDOWN, raw);
    let category_recovery = m.apply_ms(mods::OP_COOLDOWN, raw_category);
    if recovery == 0 && category_recovery == 0 {
        return;
    }
    // One that waits for its event keeps its times (an item's own) as they
    // are, the modifiers then to be had once as it starts (mangos's
    // `AddCooldown` at the aura's going, Thistle Tea's "retains item
    // overrides and applies current cooldown modifiers once").
    if s.attr(0, ATTR_COOLDOWN_ON_EVENT) {
        if let Some(c) = map.caster_mut(caster)
            && !c.pending_cooldowns.contains(&s.id)
        {
            c.pending_cooldowns.push(s.id);
            c.pending_times.retain(|p| p.0 != s.id);
            c.pending_times.push((s.id, raw, category, raw_category));
            c.cooldown_sources.insert(s.id, (item_entry, category));
        }
        return;
    }
    if let Some(c) = map.caster_mut(caster) {
        c.cooldown_sources.insert(s.id, (item_entry, category));
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
    send_to_master(map, caster, &[(s.id, ms)]);
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

/// `SMSG_SPELL_COOLDOWN` for a creature's cooldowns, under its own GUID, to
/// the player whose pet, charm or possession it is, for its bar (mangos's
/// `Creature::AddCooldown`, Thistle Tea's cooldowns of a charmed mob).
fn send_to_master(map: &Map, unit: Guid, cooldowns: &[(u32, u32)]) {
    use crate::game::map::pets::Kind;
    let Some(master) = crate::game::map::pets::state(map, unit)
        .filter(|s| !matches!(s.kind, Kind::Guardian | Kind::Totem | Kind::Critter))
        .map(|s| s.owner)
    else {
        return;
    };
    if let Some(p) = map.players.get(&master)
        && !cooldowns.is_empty()
    {
        p.outbound.send(&SMSG_SPELL_COOLDOWN {
            guid: unit,
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
    let times = c
        .pending_times
        .iter()
        .position(|p| p.0 == spell)
        .map(|i| c.pending_times.remove(i));
    if !start {
        send_clear(map, target, spell);
        return;
    }
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    // The times the cast had (an item's own), else the spell's.
    let (recovery, category, category_recovery) = match times {
        Some((_, r, c, cr)) => (r, c, cr),
        None => (
            charm_cooldown(map, target, spell)
                .unwrap_or(s.recovery_time.max(s.category_recovery_time)),
            0,
            0,
        ),
    };
    // With the modifiers its caster has now.
    let m = mods::for_spell(map, target, s);
    let recovery = m.apply_ms(mods::OP_COOLDOWN, recovery);
    let category_recovery = m.apply_ms(mods::OP_COOLDOWN, category_recovery);
    // The spell itself waits the longer of the two.
    let longer = recovery.max(category_recovery);
    if let Some(c) = map.caster_mut(target) {
        if longer > 0 {
            c.cooldowns
                .insert(spell, now + Duration::from_millis(u64::from(longer)));
        }
        if category != 0 && category_recovery > 0 {
            c.category_cooldowns.insert(
                category,
                now + Duration::from_millis(u64::from(category_recovery)),
            );
        }
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
    let until = now + Duration::from_millis(u64::from(ms));
    if let Some(c) = map.caster_mut(target) {
        c.school_lockout.push((mask, until));
    }
    let Some(p) = map.players.get(&target) else {
        return;
    };
    // The client is told of the school's spells the lockout holds longer
    // than they were held: not one whose own or category cooldown runs
    // past it, nor one whose cooldown waits for its event (mangos's
    // `Player::LockOutSpells`, Thistle Tea's "preserves longer spell and
    // category cooldowns and deferred entries").
    let c = &p.caster;
    let longer = |t: &SpellTemplate| {
        c.cooldowns.get(&t.id).is_some_and(|e| *e >= until)
            || (t.category != 0
                && c.category_cooldowns
                    .get(&t.category)
                    .is_some_and(|e| *e >= until))
    };
    let locked: Vec<(u32, u32)> = p
        .spells
        .iter()
        .filter_map(|id| data.spells.get(*id))
        .filter(|t| {
            !t.passive()
                && t.school_mask() & mask != 0
                && !t.melee_or_ranged()
                && !t.attr(0, ATTR_COOLDOWN_ON_EVENT)
                && !longer(t)
        })
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

/// Where the cooldowns a player keeps came from: (spell, the item's entry
/// or 0, the category), for those still cooling.
pub fn saved_sources(p: &Player, now: Instant) -> Vec<(u32, u32, u32)> {
    let c = &p.caster;
    let mut v: Vec<(u32, u32, u32)> = c
        .cooldown_sources
        .iter()
        .filter(|(id, (_, category))| {
            c.cooldowns.get(id).is_some_and(|t| *t > now)
                || c.category_cooldowns.get(category).is_some_and(|t| *t > now)
        })
        .map(|(id, (item, category))| (*id, *item, *category))
        .collect();
    v.sort_unstable();
    v
}

/// How long the player's saved auras and cooldowns were away, in
/// milliseconds: since they were saved, else since the logout; never less
/// than nothing, so that a clock put back lengthens nothing.
pub fn away_ms(p: &Player) -> u64 {
    let saved = match p.row.state.saved_at_ms {
        0 => p.row.logout_time.saturating_mul(1000),
        ms => ms,
    };
    if saved <= 0 {
        return 0;
    }
    u64::try_from(chrono::Utc::now().timestamp_millis() - saved).unwrap_or(0)
}

/// What is left of a saved aura of `s` after `away` milliseconds away: a
/// harmful aura's time runs while its player is away, as does one that
/// says so (`SPELL_ATTR_EX4_AURA_EXPIRES_OFFLINE`); a helpful one waits
/// (mangos's `_LoadAuras`). `None` if it has run out.
pub fn left_after(s: &SpellTemplate, left: Option<u32>, away: u64) -> Option<Option<u32>> {
    const ATTR_EX4_AURA_EXPIRES_OFFLINE: u32 = 0x4;
    let Some(ms) = left else {
        return Some(None);
    };
    if !s.attr(4, ATTR_EX4_AURA_EXPIRES_OFFLINE) && auras::positive(s) {
        return Some(Some(ms));
    }
    let ms = u64::from(ms).saturating_sub(away);
    (ms > 0).then(|| Some(ms as u32))
}

/// The cooldowns a player saved come back into their caster, before the
/// client is told of them in `SMSG_INITIAL_SPELLS`: with their items and
/// categories, and a cooldown waiting for good for each saved aura of the
/// player's own whose cooldown waits for it to go (mangos's
/// `_LoadSpellCooldowns`, and `_LoadAuras`'s `AddCooldown(.., true)`).
pub fn load(p: &mut Player, data: &crate::game::data::GameData, now: Instant) {
    let away = away_ms(p);
    let cooldowns = std::mem::take(&mut p.row.state.cooldowns);
    let categories = std::mem::take(&mut p.row.state.category_cooldowns);
    let sources = std::mem::take(&mut p.row.state.cooldown_sources);
    // A cooldown's time runs while its player is away: mangos keeps when
    // it ends (`_SaveSpellCooldowns`, `_LoadSpellCooldowns`).
    for (id, ms) in cooldowns {
        let ms = u64::from(ms).saturating_sub(away);
        if ms > 0 {
            p.caster
                .cooldowns
                .insert(id, now + Duration::from_millis(ms));
        }
    }
    for (category, ms) in categories {
        let ms = u64::from(ms).saturating_sub(away);
        if ms > 0 {
            p.caster
                .category_cooldowns
                .insert(category, now + Duration::from_millis(ms));
        }
    }
    for (id, item, category) in sources {
        p.caster.cooldown_sources.insert(id, (item, category));
    }
    let own = p.guid.guid();
    for a in &p.row.state.auras {
        let Some(s) = data.spells.get(a.spell) else {
            continue;
        };
        if s.attr(0, ATTR_COOLDOWN_ON_EVENT)
            && (a.caster == 0 || a.caster == own)
            && !p.caster.pending_cooldowns.contains(&s.id)
        {
            p.caster.pending_cooldowns.push(s.id);
            p.caster.cooldown_sources.insert(s.id, (0, s.category));
        }
    }
}

/// `SMSG_INITIAL_SPELLS`'s cooldowns, as mangos's `SendInitialSpells` has
/// them: one a spell (known or not: an item's), with the item that cast
/// it, its category and what is left of each; one waiting for its event
/// is a millisecond and the category's `0x80000000`. A category cooling
/// with no spell here to carry it goes under a known spell of it.
pub fn initial(p: &Player, data: &crate::game::data::GameData, now: Instant) -> Vec<CooldownSpell> {
    const WAITING: u32 = 0x8000_0000;
    let c = &p.caster;
    let left = |t: Option<&Instant>| {
        t.filter(|t| **t > now)
            .map_or(0, |t| t.saturating_duration_since(now).as_millis() as u32)
    };
    let mut ids: Vec<u32> = c
        .cooldowns
        .keys()
        .chain(c.cooldown_sources.keys())
        .chain(c.pending_cooldowns.iter())
        .copied()
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let entry = |id: u32, item: u32, category: u32, spell_ms: u32, category_ms: u32| {
        Some(CooldownSpell {
            spell_id: u16::try_from(id).ok()?,
            item_id: u16::try_from(item).unwrap_or(0),
            spell_category: u16::try_from(category).unwrap_or(0),
            cooldown: Duration::from_millis(u64::from(spell_ms)),
            category_cooldown: Duration::from_millis(u64::from(category_ms)),
        })
    };
    let mut told: Vec<u32> = Vec::new();
    let mut out = Vec::new();
    for id in ids {
        let (item, category) = c
            .cooldown_sources
            .get(&id)
            .copied()
            .unwrap_or_else(|| (0, data.spells.get(id).map_or(0, |s| s.category)));
        let mut spell_ms = left(c.cooldowns.get(&id));
        let mut category_ms = if category != 0 && !told.contains(&category) {
            left(c.category_cooldowns.get(&category))
        } else {
            0
        };
        if category_ms > 0 {
            told.push(category);
        }
        if c.pending_cooldowns.contains(&id) {
            spell_ms = 1;
            category_ms |= WAITING;
        }
        if spell_ms == 0 && category_ms == 0 {
            continue;
        }
        out.extend(entry(id, item, category, spell_ms, category_ms));
    }
    let mut rest: Vec<(u32, u32)> = c
        .category_cooldowns
        .iter()
        .filter(|(category, _)| !told.contains(category))
        .map(|(category, t)| (*category, left(Some(t))))
        .filter(|(_, ms)| *ms > 0)
        .collect();
    rest.sort_unstable();
    for (category, ms) in rest {
        if let Some(id) = p
            .spells
            .iter()
            .copied()
            .filter(|id| data.spells.get(*id).is_some_and(|s| s.category == category))
            .min()
        {
            out.extend(entry(id, 0, category, 0, ms));
        }
    }
    out
}

/// A player's saved auras and cooldowns come back at login (the client
/// told of the cooldowns in `SMSG_INITIAL_SPELLS`, [`initial`]).
pub fn restore(map: &mut Map, player: Guid) {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    load(p, &data, now);
    let away = away_ms(p);
    let saved = std::mem::take(&mut p.row.state.auras);
    for a in saved {
        let Some(s) = data.spells.get(a.spell) else {
            continue;
        };
        let Some(left) = left_after(s, a.left, away) else {
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
        let item = (a.item != 0).then(|| Guid::new(a.item));
        // Its ticks as its caster's modifiers have them now.
        let m = super::mods::for_spell(map, caster, s);
        let _ = auras::apply_spell(
            map,
            caster,
            player,
            s,
            &effects,
            auras::Context {
                item,
                equipped: false,
                combo: a.combo,
                duration: Some(left),
                charges: Some(a.charges),
                triggered: a.triggered,
                linked: false,
                activation: (
                    m.flat[super::mods::OP_ACTIVATION_TIME],
                    m.pct[super::mods::OP_ACTIVATION_TIME],
                ),
            },
        );
        // Its stacks, on its bar and in the stats.
        auras::set_stacks(map, player, a.spell, caster, item, a.stacks.max(1));
    }
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
