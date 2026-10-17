//! Players under another's control, and whose movement a client sends.
//!
//! A player possessed by another (Mind Control) loses control of their
//! character: their possessor's client moves it, once it has made it its
//! active mover, and commands it from the possess bar; it fights on its
//! possessor's side. A player charmed by a creature fights for the
//! creature on its own: its melee, or its harmful spells (Thistle Tea's
//! `PlayerCharm`). Either ends when the aura goes, or either of them dies
//! or leaves, and the player has their own character back.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::*;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_MOD_CHARM, AURA_MOD_POSSESS};
use crate::game::map::combat::dist;

/// How often a charmed player looks for a fight.
const CHARM_THINK: Duration = Duration::from_millis(500);
/// How far a charmed player looks.
const CHARM_YARDS: f32 = 30.0;

/// Who controls a player, and what they were before.
#[derive(Debug, Clone, Copy)]
pub struct Possession {
    pub master: Guid,
    pub spell: u32,
    pub was_faction: u32,
    /// Moved by the master's client (possession), or fighting on its own
    /// (a creature's charm).
    pub manual: bool,
    pub next_think: Instant,
}

/// The spell of the aura by which `master` holds `unit`.
fn holding_spell(map: &Map, master: Guid, unit: Guid) -> Option<u32> {
    crate::game::map::auras::auras(map, unit).and_then(|a| {
        a.holders
            .iter()
            .find(|h| h.caster == master && (h.has(AURA_MOD_POSSESS) || h.has(AURA_MOD_CHARM)))
            .map(|h| h.spell)
    })
}

/// The spells a charmed player is given to cast: their harmful class
/// spells, each at its best rank (Thistle Tea's `PlayerCharm.spells`).
fn charm_spells(map: &Map, player: Guid) -> Vec<u32> {
    let data = &map.realm.data;
    let Some(p) = map.players.get(&player) else {
        return Vec::new();
    };
    let mut best: Vec<(u32, u32, u32)> = Vec::new();
    for s in p.spells.iter().filter_map(|s| data.spells.get(*s)) {
        let usable = s.family_name != 0
            && !crate::game::map::auras::positive(s)
            && !s.passive()
            && s.aura_interrupt_flags & crate::game::map::auras::INTERRUPT_DAMAGE == 0
            && s.attributes[1] & 0x2_0000 == 0;
        if !usable {
            continue;
        }
        let first = data.spells.first_rank(s.id);
        match best.iter_mut().find(|(f, _, _)| *f == first) {
            Some(b) if b.1 < s.spell_level => *b = (first, s.spell_level, s.id),
            Some(_) => {}
            None => best.push((first, s.spell_level, s.id)),
        }
    }
    let mut spells: Vec<u32> = best.into_iter().map(|(_, _, id)| id).collect();
    spells.sort_unstable();
    spells
}

/// `master` takes control of player `victim`.
pub fn take(map: &mut Map, master: Guid, victim: Guid) {
    if map
        .players
        .get(&victim)
        .is_none_or(|p| p.owned.possessed_by.is_some())
    {
        return;
    }
    let now = map.now;
    let manual = guid::is_player(master);
    let spell = holding_spell(map, master, victim).unwrap_or(0);
    let faction = map
        .unit_fields(master)
        .map(|f| f.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
        .unwrap_or(0);
    crate::game::map::spells::interrupt(map, victim);
    crate::game::map::combat::leave_combat(map, victim);
    crate::game::map::ai::forget(map, victim);
    let Some(p) = map.players.get_mut(&victim) else {
        return;
    };
    p.owned.possessed_by = Some(Possession {
        master,
        spell,
        was_faction: p.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
        manual,
        next_think: now,
    });
    p.fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, faction);
    p.fields.set_guid(UNIT_FIELD_CHARMEDBY, master);
    p.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_POSSESSED, true);
    // Their own client lets go of the character.
    p.outbound.send(&SMSG_CLIENT_CONTROL_UPDATE {
        guid: victim,
        allow_movement: false,
    });
    if let Some(m) = map.players.get_mut(&master) {
        m.owned.charm = Some(victim);
        m.fields.set_guid(UNIT_FIELD_CHARM, victim);
        m.fields.set_guid(PLAYER_FARSIGHT, victim);
        m.outbound.send(&SMSG_CLIENT_CONTROL_UPDATE {
            guid: victim,
            allow_movement: true,
        });
    }
    if manual {
        send_spells(map, master);
    }
}

/// The bar a possessor commands a player from: attack, and the player's
/// harmful spells.
pub fn bar(map: &Map, victim: Guid) -> [u32; 10] {
    let spells = charm_spells(map, victim);
    control::default_bar(&map.realm.data, &spells, &[], true)
}

/// Player `victim` is theirs again.
pub fn release(map: &mut Map, victim: Guid) {
    let Some(pos) = map
        .players
        .get_mut(&victim)
        .and_then(|p| p.owned.possessed_by.take())
    else {
        return;
    };
    crate::game::map::combat::stop_attack(map, victim);
    crate::game::map::spells::interrupt(map, victim);
    if let Some(p) = map.players.get_mut(&victim) {
        p.fields
            .set_u32(UNIT_FIELD_FACTIONTEMPLATE, pos.was_faction);
        p.fields.set_guid(UNIT_FIELD_CHARMEDBY, Guid::zero());
        p.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_POSSESSED, false);
        p.outbound.send(&SMSG_CLIENT_CONTROL_UPDATE {
            guid: victim,
            allow_movement: true,
        });
    }
    if let Some(m) = map.players.get_mut(&pos.master) {
        if m.owned.charm == Some(victim) {
            m.owned.charm = None;
        }
        if m.owned.mover == Some(victim) {
            m.owned.mover = None;
        }
        m.fields.set_guid(UNIT_FIELD_CHARM, Guid::zero());
        m.fields.set_guid(PLAYER_FARSIGHT, Guid::zero());
        m.outbound.send(&SMSG_CLIENT_CONTROL_UPDATE {
            guid: pos.master,
            allow_movement: true,
        });
    }
    if guid::is_player(pos.master) {
        send_spells(map, pos.master);
    }
    if pos.spell != 0 {
        crate::game::map::auras::remove_spell(map, victim, pos.spell, Some(pos.master));
    }
}

/// `CMSG_SET_ACTIVE_MOVER`: the client moves `mover` now (itself, or the
/// unit it possesses).
pub fn set_mover(map: &mut Map, player: Guid, mover: Guid) {
    let possessed = map
        .players
        .get(&player)
        .and_then(|p| p.owned.charm)
        .filter(|c| *c == mover);
    let manual = possessed.is_some_and(|c| {
        state(map, c).is_some_and(|s| s.kind == Kind::Possessed)
            || map
                .players
                .get(&c)
                .and_then(|v| v.owned.possessed_by)
                .is_some_and(|pos| pos.manual && pos.master == player)
    });
    let manual = manual || super::eyes::controls(map, player, mover);
    if let Some(p) = map.players.get_mut(&player) {
        p.owned.mover = (manual && mover != player).then_some(mover);
    }
}

/// `CMSG_MOVE_NOT_ACTIVE_MOVER`: the client no longer moves `old`.
pub fn not_mover(map: &mut Map, player: Guid, old: Guid) {
    if let Some(p) = map.players.get_mut(&player)
        && p.owned.mover == Some(old)
    {
        p.owned.mover = None;
    }
}

/// Movement from a client: a possessed unit's, when it is the client's
/// active mover (true when handled here); a possessed player's own client
/// moves nothing.
pub fn moved(
    map: &mut Map,
    player: Guid,
    info: &MovementInfo,
    relay: crate::game::map::movement::Relay,
) -> bool {
    let Some(p) = map.players.get(&player) else {
        return false;
    };
    if p.owned.possessed_by.is_some() {
        // Their character is someone else's to move.
        return true;
    }
    let Some(unit) = p.owned.mover else {
        return false;
    };
    if guid::is_player(unit) {
        let Some(v) = map.players.get_mut(&unit) else {
            return true;
        };
        let (anchor, old_cell) = (v.vis_anchor, v.cell);
        v.movement = info.clone();
        let new_cell = crate::game::map::grid::cell_of(info.position.x, info.position.y);
        v.cell = new_cell;
        v.move_check = Some((info.position, map.now));
        map.grid.relocate(unit, old_cell, new_cell);
        crate::game::map::visibility::moved(map, unit, anchor, info.position);
        if let Some(frame) = relay(unit, info.clone()) {
            // Its own client sees it walk too.
            map.send_to_observers(unit, &frame, true);
        }
        return true;
    }
    let Some(c) = map.creature_mut(unit) else {
        return true;
    };
    let (anchor, old_cell) = (c.vis_anchor, c.cell);
    c.position = info.position;
    c.orientation = info.orientation;
    c.motion = crate::game::map::creature::Motion::Idle { until: None };
    let new_cell = crate::game::map::grid::cell_of(info.position.x, info.position.y);
    c.cell = new_cell;
    map.grid.relocate(unit, old_cell, new_cell);
    crate::game::map::visibility::moved(map, unit, anchor, info.position);
    if let Some(frame) = relay(unit, info.clone()) {
        map.send_to_observers(unit, &frame, false);
    }
    true
}

/// The players under control: still held, and (charmed by a creature)
/// fighting for it.
pub fn update(map: &mut Map) {
    let now = map.now;
    let held: Vec<(Guid, Possession)> = map
        .players
        .values()
        .filter_map(|p| p.owned.possessed_by.map(|pos| (p.guid, pos)))
        .collect();
    for (victim, pos) in held {
        let master_alive = map.unit_view(pos.master).is_some_and(|u| u.alive);
        let victim_alive = map.unit_view(victim).is_some_and(|u| u.alive);
        let aura = pos.spell == 0 || holding_spell(map, pos.master, victim) == Some(pos.spell);
        if !master_alive || !victim_alive || !aura {
            release(map, victim);
            continue;
        }
        if !pos.manual && pos.next_think <= now {
            if let Some(p) = map.players.get_mut(&victim)
                && let Some(x) = p.owned.possessed_by.as_mut()
            {
                x.next_think = now + CHARM_THINK;
            }
            fight(map, victim, pos.master);
        }
    }
}

/// A charmed player's turn: the nearest of its charmer's foes, by spell or
/// by blade.
fn fight(map: &mut Map, victim: Guid, master: Guid) {
    let Some(at) = map.position_of(victim) else {
        return;
    };
    let current = map
        .combat(victim)
        .filter(|c| c.attacking)
        .map(|c| c.victim)
        .filter(|t| crate::game::map::combat::can_attack(map, victim, *t));
    let target = current.or_else(|| {
        map.players_near(at.x, at.y, CHARM_YARDS)
            .into_iter()
            .filter(|g| *g != victim && *g != master)
            .filter(|g| crate::game::map::combat::can_attack(map, victim, *g))
            .min_by(|a, b| {
                let da = map.position_of(*a).map_or(f32::MAX, |p| dist(p, at));
                let db = map.position_of(*b).map_or(f32::MAX, |p| dist(p, at));
                da.total_cmp(&db)
            })
    });
    let Some(target) = target else {
        return;
    };
    let melee = map
        .players
        .get(&victim)
        .is_some_and(|p| matches!(p.row.class, 1 | 2 | 4 | 11));
    if !melee && map.caster(victim).is_some_and(|c| c.casting.is_none()) {
        let data = map.realm.data.clone();
        let d = map.position_of(target).map_or(f32::MAX, |p| dist(p, at));
        let spell = charm_spells(map, victim).into_iter().find(|s| {
            data.spells
                .get(*s)
                .is_some_and(|t| d <= data.spells.range(t).1)
        });
        if let Some(spell) = spell {
            crate::game::map::spells::cast(
                map,
                victim,
                spell,
                crate::game::map::spells::Targets {
                    unit: Some(target),
                    ..Default::default()
                },
                None,
            );
            return;
        }
    }
    crate::game::map::combat::start_attack(map, victim, target);
}
