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
use crate::game::map::auras::{AURA_AOE_CHARM, AURA_MOD_CHARM, AURA_MOD_POSSESS};
use rand::RngExt;

/// How often a charmed player looks for a fight.
const CHARM_THINK: Duration = Duration::from_millis(500);
/// How often a creature-charmed player tries a spell (Thistle Tea's
/// `BT.Charm.try_spell`).
const CHARM_CAST_EVERY: Duration = Duration::from_millis(1500);

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
    /// When a creature-charmed player next tries a spell, and whether it
    /// fights by blade till then, its last try having nothing to cast.
    pub next_cast_at: Instant,
    pub melee_fallback: bool,
}

/// The spell of the aura by which `master` holds `unit`.
fn holding_spell(map: &Map, master: Guid, unit: Guid) -> Option<u32> {
    crate::game::map::auras::auras(map, unit).and_then(|a| {
        a.holders
            .iter()
            .find(|h| {
                h.caster == master
                    && (h.has(AURA_MOD_POSSESS) || h.has(AURA_MOD_CHARM) || h.has(AURA_AOE_CHARM))
            })
            .map(|h| h.spell)
    })
}

/// The spells a charmed player is given to cast: their harmful class
/// spells shown in the spellbook, each at its best rank (Thistle Tea's
/// `PlayerCharm.spells`).
pub(crate) fn charm_spells(map: &Map, player: Guid) -> Vec<u32> {
    let data = &map.realm.data;
    let Some(p) = map.players.get(&player) else {
        return Vec::new();
    };
    let mut best: Vec<(u32, u32, u32)> = Vec::new();
    for s in p.spells.iter().filter_map(|s| data.spells.get(*s)) {
        let usable = s.family_name != 0
            && !crate::game::map::auras::positive(s)
            && !s.passive()
            && !s.attr(0, crate::game::data::spells::ATTR_HIDDEN)
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
        next_cast_at: now,
        melee_fallback: false,
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
    // A creature's charm moves the player on the server.
    crate::game::map::player_motion::control(map, victim);
}

/// The bar a possessor commands a player from: attack, and nothing else
/// (mangos's `InitPossessCreateSpells`: "possessed players don't have
/// spells"; Thistle Tea's `SmsgPetSpells.for_possession`).
pub fn bar(_map: &Map, _victim: Guid) -> [u32; 10] {
    let mut bar = [control::button(0, ACT_PASSIVE); 10];
    bar[0] = control::button(u32::from(COMMAND_ATTACK), ACT_COMMAND);
    bar
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
    // Under a fear or a confusion its client has no control back (mangos's
    // `UpdateClientControl` checks `IsClientControlled`): the server moves
    // it still, and gives it back when that ends.
    let driven = matches!(
        crate::game::map::player_motion::driven_by(map, victim),
        Some(
            crate::game::map::player_motion::Mode::Fear
                | crate::game::map::player_motion::Mode::Confusion { .. }
        )
    );
    if let Some(p) = map.players.get_mut(&victim) {
        p.fields
            .set_u32(UNIT_FIELD_FACTIONTEMPLATE, pos.was_faction);
        p.fields.set_guid(UNIT_FIELD_CHARMEDBY, Guid::zero());
        p.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_POSSESSED, false);
        if !driven {
            // Where its possessor left it is where its client starts again.
            let at = p.position();
            crate::game::map::move_guard::regained(p, at, map.now, &map.cfg.anticheat);
            p.outbound.send(&SMSG_CLIENT_CONTROL_UPDATE {
                guid: victim,
                allow_movement: true,
            });
        }
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
    crate::game::map::player_motion::control(map, victim);
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
    // One still theirs to move stays their mover: control changes with
    // `CMSG_SET_ACTIVE_MOVER` (mangos's `HandleMoveNotActiveMover`
    // ignores the mover's own guid; Thistle Tea's "a remote mover that
    // remains controlled").
    let controlled = map
        .players
        .get(&player)
        .is_some_and(|p| p.owned.charm == Some(old))
        || super::eyes::controls(map, player, old);
    if controlled {
        return;
    }
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
    // Checked as the controller's own movement is: a NaN or far-off place
    // would be told to everyone who sees the unit, and saved with it.
    if !crate::game::map::movement::finite(info) {
        tracing::debug!("{}'s client moved {unit:?} out of the world", p.name);
        return true;
    }
    if guid::is_player(unit) {
        moved_player(map, player, unit, *info, relay);
    } else {
        moved_creature(map, player, unit, *info, relay);
    }
    true
}

/// A possessed player moved by `player`'s client: checked as their own
/// client's movement is, against their speeds, roots and auras, with their
/// own guard (mangos-classic's `VerifyMovementInfo` of the mover).
fn moved_player(
    map: &mut Map,
    player: Guid,
    unit: Guid,
    info: MovementInfo,
    relay: crate::game::map::movement::Relay,
) {
    use crate::game::map::move_guard::{self, Mover};
    let walks_on_water = crate::game::map::death::walks_on_water(map, unit);
    let now = map.now;
    let check = map.cfg.anticheat.clone();
    let (Some(v), Some(p)) = (map.players.get(&unit), map.players.get(&player)) else {
        return;
    };
    // Feared or confused, the server moves them, as a creature; dead,
    // no one does.
    if v.driven.is_some() || v.death != crate::game::map::death::Death::Alive {
        return;
    }
    // The controller's client says it: their GM status is what counts.
    let mover = Mover {
        gm: p.gm,
        ..Mover::player_moved_by(v, &p.outbound)
    };
    let mut guard = v.misc.move_guard;
    let (at, o) = (v.position(), v.movement.orientation);
    // Held by a root or stun: refused, and put back if it went anywhere.
    if move_guard::held_back_of(&mover, &mut guard, &info, now) {
        let far = horizontal(at, info.position) > check.distance_allowance_yards;
        if let Some(v) = map.players.get_mut(&unit) {
            v.misc.move_guard = guard;
        }
        if far {
            tracing::debug!("{unit:?} was moved under a root by {player:?}; put back");
            crate::game::map::movement::teleport_within(map, unit, at, o);
        }
        return;
    }
    let info = move_guard::stripped_of(&mover, &guard, info, walks_on_water);
    if let Some((last, since)) = v.move_check
        && check.speed_check
        && !p.gm
        && info.flags.get_on_transport().is_none()
        && move_guard::too_fast_at(v.speeds, &mut guard, last, since, &info, now, &check)
    {
        tracing::debug!("{unit:?} was moved too fast by {player:?}; pulled back");
        crate::game::map::movement::teleport_within(map, unit, last, o);
        return;
    }
    move_guard::taken_with(&mut guard, &info);
    let Some(v) = map.players.get_mut(&unit) else {
        return;
    };
    v.misc.move_guard = guard;
    let (anchor, old_cell) = (v.vis_anchor, v.cell);
    v.movement = info;
    let new_cell = crate::game::map::grid::cell_of(info.position.x, info.position.y);
    v.cell = new_cell;
    v.move_check = Some((info.position, now));
    map.grid.relocate(unit, old_cell, new_cell);
    crate::game::map::visibility::moved(map, unit, anchor, info.position);
    if let Some(frame) = relay(unit, info) {
        // Its own client sees it walk too; the controller's, which
        // moved it, isn't told back (mangos's `HandleMovementOpcodes`,
        // `SendMessageToAllWhoSeeMeMove(data, _player)`).
        map.send_to_observers_but(unit, &frame, true, player);
    }
}

/// A possessed creature (Mind Control, an eye, Eyes of the Beast) moved by
/// `player`'s client: checked against its speeds, roots and auras, with a
/// guard of its own while it is controlled.
fn moved_creature(
    map: &mut Map,
    player: Guid,
    unit: Guid,
    info: MovementInfo,
    relay: crate::game::map::movement::Relay,
) {
    use crate::game::map::move_guard::{self, Mover};
    let walks_on_water = crate::game::map::death::walks_on_water(map, unit);
    let now = map.now;
    let check = map.cfg.anticheat.clone();
    let (Some(c), Some(p)) = (map.creature(unit), map.players.get(&player)) else {
        return;
    };
    // Feared or confused, the server moves it (Thistle Tea's "possessed
    // creatures reject input"); dead, no one does.
    if c.engagement.control.is_some() || !c.alive() {
        return;
    }
    let Some(s) = c.pet.as_deref() else {
        return;
    };
    let mover = Mover {
        guid: unit,
        name: "",
        position: c.position,
        auras: &c.auras,
        speeds: c.speeds,
        orders: &p.outbound,
        gm: p.gm,
    };
    let mut guard = s.move_guard;
    let (at, o) = (c.position, c.orientation);
    let since = s.moved_at.unwrap_or(now);
    let held = move_guard::held_back_of(&mover, &mut guard, &info, now);
    let info = move_guard::stripped_of(&mover, &guard, info, walks_on_water);
    let fast = !held
        && check.speed_check
        && !p.gm
        && info.flags.get_on_transport().is_none()
        && move_guard::too_fast_at(c.speeds, &mut guard, at, since, &info, now, &check);
    if held || fast {
        let far = horizontal(at, info.position) > check.distance_allowance_yards;
        if let Some(s) = super::state_mut(map, unit) {
            if fast {
                guard.refill();
            }
            s.move_guard = guard;
        }
        // Where the server has it, told to those who see it.
        if fast || far {
            tracing::debug!("{unit:?} was moved past its checks by {player:?}; put back");
            crate::game::map::scripts::place(map, unit, at, o);
        }
        return;
    }
    move_guard::taken_with(&mut guard, &info);
    let Some(c) = map.creature_mut(unit) else {
        return;
    };
    if let Some(s) = c.pet.as_deref_mut() {
        s.move_guard = guard;
        s.moved_at = Some(now);
    }
    let (anchor, old_cell) = (c.vis_anchor, c.cell);
    c.position = info.position;
    c.orientation = info.orientation;
    c.motion = crate::game::map::creature::Motion::Idle { until: None };
    let new_cell = crate::game::map::grid::cell_of(info.position.x, info.position.y);
    c.cell = new_cell;
    map.grid.relocate(unit, old_cell, new_cell);
    crate::game::map::visibility::moved(map, unit, anchor, info.position);
    if let Some(frame) = relay(unit, info) {
        map.send_to_observers_but(unit, &frame, false, player);
    }
}

fn horizontal(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
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
        // A creature's charm lasts while it fights (Thistle Tea's
        // `PlayerCharm.controller_active?`).
        let fighting = pos.manual
            || map.creature(pos.master).is_some_and(|c| {
                c.fields.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                )
            });
        if !master_alive || !victim_alive || !aura || !fighting {
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

/// Whether a charmed player may fight `t` for `master`: not itself, its
/// charmer or one its charmer owns; alive, seen and attackable.
fn charm_target(map: &Map, victim: Guid, master: Guid, t: Guid) -> bool {
    t != victim
        && t != master
        && crate::game::map::pets::master(map, t) != master
        && map.unit_view(t).is_some_and(|u| u.alive)
        && crate::game::map::spells::stealth::sees(map, victim, t)
        && crate::game::map::combat::can_attack(map, victim, t)
}

/// A charmed player's turn (Thistle Tea's `BT.Charm`): the foe it fights
/// while it may, else one of its charmer's foes at random; every second
/// and a half a spell of its own at random, if it can cast it there, else
/// its blade till the next try. With no foe it follows.
fn fight(map: &mut Map, victim: Guid, master: Guid) {
    let now = map.now;
    // Stunned, it does nothing (Thistle Tea's "stuns suspend the charm
    // AI").
    if crate::game::map::auras::auras(map, victim)
        .is_some_and(|a| a.has(crate::game::map::auras::AURA_MOD_STUN))
    {
        crate::game::map::combat::stop_attack(map, victim);
        return;
    }
    // The foe she fights while she may: the one she strikes, or the one she
    // has selected (a caster casts from afar), whoever her charmer turns to
    // (Thistle Tea's "retains a valid victim after the controller changes
    // targets").
    let current = map
        .combat(victim)
        .filter(|c| c.attacking)
        .map(|c| c.victim)
        .or_else(|| {
            map.unit_fields(victim)
                .map(|f| f.get_guid(UNIT_FIELD_TARGET))
        })
        .filter(|t| !t.is_zero() && charm_target(map, victim, master, *t));
    let target = current.or_else(|| {
        let foes: Vec<Guid> = map
            .creature(master)
            .map(|c| c.threat.iter().map(|(t, _)| *t).collect())
            .unwrap_or_default();
        let foes: Vec<Guid> = foes
            .into_iter()
            .filter(|t| charm_target(map, victim, master, *t))
            .collect();
        (!foes.is_empty()).then(|| foes[rand::rng().random_range(0..foes.len())])
    });
    let Some(target) = target else {
        crate::game::map::combat::stop_attack(map, victim);
        if let Some(f) = map.unit_fields_mut(victim) {
            f.set_guid(UNIT_FIELD_TARGET, Guid::zero());
        }
        return;
    };
    // Selected, as she casts at it or strikes it (Thistle Tea's
    // `unit.target`).
    if let Some(f) = map.unit_fields_mut(victim) {
        f.set_guid(UNIT_FIELD_TARGET, target);
    }
    if map.caster(victim).is_some_and(|c| c.casting.is_some()) {
        return;
    }
    let Some(pos) = map.players.get(&victim).and_then(|p| p.owned.possessed_by) else {
        return;
    };
    if pos.next_cast_at <= now {
        let spells = charm_spells(map, victim);
        let spell = (!spells.is_empty()).then(|| spells[rand::rng().random_range(0..spells.len())]);
        let targets = crate::game::map::spells::Targets {
            unit: Some(target),
            ..Default::default()
        };
        let data = map.realm.data.clone();
        let castable = spell.and_then(|s| data.spells.get(s)).is_some_and(|s| {
            let m = crate::game::map::spells::mods::for_spell(map, victim, s);
            crate::game::map::spells::checks::check(map, victim, s, &targets, None, false, &m)
                .is_none()
        });
        if let Some(p) = map.players.get_mut(&victim)
            && let Some(x) = p.owned.possessed_by.as_mut()
        {
            x.next_cast_at = now + CHARM_CAST_EVERY;
            x.melee_fallback = !castable;
        }
        if castable && let Some(spell) = spell {
            crate::game::map::spells::cast(map, victim, spell, targets, None);
            return;
        }
    }
    let melee = map.players.get(&victim).is_some_and(|p| {
        matches!(p.row.class, 1 | 2 | 4 | 11)
            || p.owned.possessed_by.is_some_and(|x| x.melee_fallback)
    });
    let near = match (map.unit_view(victim), map.unit_view(target)) {
        (Some(a), Some(b)) => crate::game::map::combat::in_melee_range(&a, &b),
        _ => false,
    };
    // A caster with a spell to cast stands off, but strikes one in reach.
    if melee || near {
        crate::game::map::combat::start_attack(map, victim, target);
    }
}
