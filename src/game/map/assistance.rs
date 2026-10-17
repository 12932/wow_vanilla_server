//! Creatures helping each other, as Thistle Tea has them
//! (`Combat.Assistance`, `World.Combat.CallForHelp`): a creature drawn into
//! a fight calls those of its faction template within 10 yards that answer
//! calls, who come a second and a half later if it still fights; while it
//! fights more than 10 yards from its spawn it calls its friends within its
//! call-for-help range each second; a faction that flees from calls runs
//! from the enemy instead. And creatures' reaction modes: a passive one
//! neither looks for a fight nor takes a victim, a defensive one only
//! fights back.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::FactionTemplate;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::combat::dist;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;

/// The reach of a creature's call for help when it is drawn in.
pub const CALL_RADIUS: f32 = 10.0;
/// How long the called take to come.
pub const ASSIST_DELAY: Duration = Duration::from_millis(1500);
/// How far a fleeing creature looks for help.
pub const SEEK_RADIUS: f32 = 30.0;
/// How often a creature fighting away from its spawn calls its friends,
/// and how far from it it must be.
const PULSE_EVERY: Duration = Duration::from_secs(1);
const PULSE_FROM_SPAWN: f32 = 10.0;

const FACTION_RESPONDS: u32 = 0x01;
const FACTION_FLEES: u32 = 0x400;
/// Not attackable, stunned, not selectable: in no state to help.
const UNAVAILABLE_FLAGS: u32 = 0x0200_0000 | 0x0004_0000 | 0x2;
const EXTRA_NO_ASSIST: u32 = 0x0001_0000;
const EXTRA_PASSIVE: u32 = 0x0000_0080 | 0x0002_0000;
const EXTRA_NO_AGGRO: u32 = 0x0000_0002;
const STATIC_IGNORE_COMBAT: u32 = 0x0200_0000;

/// How a creature meets enemies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaction {
    /// It neither looks for a fight nor takes a victim.
    Passive,
    /// It fights back, but looks for no fight.
    Defensive,
    Aggressive,
}

/// A creature's reaction mode: a script's, else its template's.
pub fn reaction(map: &Map, g: Guid) -> Reaction {
    let Some(c) = map.creature(g) else {
        return Reaction::Aggressive;
    };
    if let Some(s) = c.scripted.as_deref() {
        if s.passive {
            return Reaction::Passive;
        }
        if let Some(r) = s.reaction {
            return r;
        }
    }
    let Some(t) = map.realm.data.creature_template(c.entry) else {
        return Reaction::Aggressive;
    };
    if t.flags_extra & EXTRA_PASSIVE != 0 || t.static_flags1 & STATIC_IGNORE_COMBAT != 0 {
        Reaction::Passive
    } else if t.flags_extra & EXTRA_NO_AGGRO != 0 {
        Reaction::Defensive
    } else {
        Reaction::Aggressive
    }
}

/// Whether `from` sees `to` at Thistle Tea's eye heights: two yards up for
/// the one looking, one for the one looked at.
pub fn sees(map: &mut Map, from: Vector3d, to: Vector3d) -> bool {
    // `checks::line_of_sight` raises both ends two yards.
    crate::game::map::spells::checks::line_of_sight(
        map,
        from,
        Vector3d {
            z: to.z - 1.0,
            ..to
        },
    )
}

fn faction(map: &Map, g: Guid) -> Option<&FactionTemplate> {
    let f = map.unit_view(g)?.faction;
    map.realm.data.faction_templates.get(&f)
}

/// Whether a creature fights now.
fn fighting(map: &Map, g: Guid) -> bool {
    map.creature(g).is_some_and(|c| {
        c.combat.attacking || c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT)
    })
}

/// Whether `h` is free to come when called: alive, out of a fight, its own
/// master, not a critter, not passive, in a state to fight, not on its way
/// home.
pub fn available(map: &Map, h: Guid) -> bool {
    let Some(c) = map.creature(h) else {
        return false;
    };
    let flags = c.fields.get_u32(UNIT_FIELD_FLAGS);
    let extra = map
        .realm
        .data
        .creature_template(c.entry)
        .map_or(0, |t| t.flags_extra);
    c.alive()
        && !fighting(map, h)
        && !c.evading
        && c.owner.is_none()
        && c.fields.get_guid(UNIT_FIELD_SUMMONEDBY).is_zero()
        && c.fields.get_guid(UNIT_FIELD_CHARMEDBY).is_zero()
        && flags & UNAVAILABLE_FLAGS == 0
        && flags & auras::UNIT_FLAG_PACIFIED == 0
        && extra & EXTRA_NO_ASSIST == 0
        && !c.auras.has(auras::AURA_MOD_INVISIBILITY)
        && !crate::game::map::creature_motion::critter(map, h)
        && reaction(map, h) != Reaction::Passive
}

/// Whether `helper` answers `caller`'s call against `enemy`: its faction
/// answers calls, is `caller`'s (or, `friendly`, friendly to it), and isn't
/// friendly to the enemy.
fn eligible(map: &Map, helper: Guid, caller: Guid, enemy: Guid, friendly: bool) -> bool {
    let (Some(h), Some(c), Some(e)) = (
        faction(map, helper),
        faction(map, caller),
        faction(map, enemy),
    ) else {
        return false;
    };
    let allied = h.id == c.id || (friendly && h.friendly_to(c));
    h.flags & FACTION_RESPONDS != 0
        && h.flags & FACTION_FLEES == 0
        && allied
        && !h.friendly_to(e)
        && available(map, helper)
}

/// Whether `helper`, of a faction that flees from calls, runs from `enemy`.
fn flees(map: &Map, helper: Guid, caller: Guid, enemy: Guid) -> bool {
    let (Some(h), Some(c), Some(e)) = (
        faction(map, helper),
        faction(map, caller),
        faction(map, enemy),
    ) else {
        return false;
    };
    h.flags & FACTION_FLEES != 0 && (h.id == c.id || h.friendly_to(c)) && !h.friendly_to(e)
}

/// The creatures around `caller` within `radius` that `pick` takes, in
/// sight of it.
fn around(
    map: &mut Map,
    caller: Guid,
    radius: f32,
    pick: impl Fn(&Map, Guid) -> bool,
) -> Vec<Guid> {
    let Some(at) = map.creature(caller).map(|c| c.position) else {
        return Vec::new();
    };
    let near: Vec<Guid> = map
        .grid
        .around(at.x, at.y, radius)
        .filter(|g| guid::is_unit(*g) && *g != caller)
        .filter(|g| {
            map.creature(*g)
                .is_some_and(|h| dist(h.position, at) <= radius)
        })
        .filter(|g| pick(map, *g))
        .collect();
    near.into_iter()
        .filter(|g| {
            let to = map.creature(*g).map(|c| c.position);
            to.is_some_and(|to| sees(map, at, to))
        })
        .collect()
}

/// The range a creature calls its friends from as it fights: a script's,
/// else its template's; none for a pet or totem.
fn call_range(map: &Map, g: Guid) -> f32 {
    let Some(c) = map.creature(g) else {
        return 0.0;
    };
    if c.owner.is_some() {
        return 0.0;
    }
    c.scripted
        .as_deref()
        .and_then(|s| s.call_for_help_range)
        .or_else(|| {
            map.realm
                .data
                .creature_template(c.entry)
                .map(|t| t.call_for_help_range)
        })
        .unwrap_or(0.0)
}

/// `caller` is drawn into a fight with `enemy`: if it calls for help at
/// all, those that answer within 10 yards are told to come, a moment on.
pub fn drawn_in(map: &mut Map, caller: Guid, enemy: Guid) {
    if call_range(map, caller) <= 0.0 || enemy.is_zero() {
        return;
    }
    if crate::game::map::creature_motion::critter(map, caller) {
        return;
    }
    let helpers = around(map, caller, CALL_RADIUS, |m, h| {
        eligible(m, h, caller, enemy, false)
    });
    if helpers.is_empty() {
        return;
    }
    let due = map.now + ASSIST_DELAY;
    map.rare.assist = true;
    if let Some(c) = map.creature_mut(caller) {
        c.engagement.assist.push(Assist {
            due,
            enemy,
            helpers,
        });
    }
}

/// A call for help on its way.
#[derive(Debug, Clone)]
pub struct Assist {
    pub due: Instant,
    pub enemy: Guid,
    pub helpers: Vec<Guid>,
}

/// `helper` joins the fight against `enemy`, calling no one itself.
fn join(map: &mut Map, helper: Guid, enemy: Guid) {
    if available(map, helper)
        && map.unit_view(enemy).is_some_and(|u| u.alive)
        && crate::game::map::combat::can_attack(map, helper, enemy)
    {
        crate::game::map::ai::assist(map, helper, enemy);
    }
}

/// Those called within `radius` of `caller` (its friends, too): those
/// that answer join in at once, and a faction that flees from calls runs.
pub fn call_friends(map: &mut Map, caller: Guid, enemy: Guid, radius: f32) {
    if radius <= 0.0 || enemy.is_zero() {
        return;
    }
    let helpers = around(map, caller, radius, |m, h| {
        eligible(m, h, caller, enemy, true) || flees(m, h, caller, enemy)
    });
    for h in helpers {
        if eligible(map, h, caller, enemy, true) {
            join(map, h, enemy);
        } else {
            flee_from_help(map, h, enemy);
        }
    }
}

/// A creature of a faction that flees from calls backs away from `enemy`,
/// if it is close and the creature free to: ten yards off.
fn flee_from_help(map: &mut Map, g: Guid, enemy: Guid) {
    let (Some(c), Some(e)) = (map.creature(g), map.unit_view(enemy)) else {
        return;
    };
    let free = c.alive()
        && !fighting(map, g)
        && !c.evading
        && c.movement_type == crate::game::map::creature::MOVEMENT_RANDOM
        && c.scripted
            .as_deref()
            .is_none_or(|s| s.scripted_move.is_none());
    let theirs = map
        .unit_fields(enemy)
        .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS).max(0.0));
    let reach = c.fields.get_f32(UNIT_FIELD_BOUNDINGRADIUS).max(0.0) + theirs;
    if !free || dist(c.position, e.position) - reach >= 10.0 {
        return;
    }
    crate::game::map::ai::back_away(map, g, enemy, 10.0);
}

/// The calls for help due, and the friends called by those fighting away
/// from their spawns.
pub fn update(map: &mut Map) {
    let now = map.now;
    let mut due: Vec<(Guid, Assist)> = Vec::new();
    if map.rare.assist {
        let mut any = false;
        due = map
            .creatures
            .iter_mut()
            .filter(|(_, c)| {
                any |= !c.engagement.assist.is_empty();
                c.engagement.assist.iter().any(|a| a.due <= now)
            })
            .flat_map(|(_, c)| {
                let g = c.guid;
                let (ready, wait): (Vec<Assist>, Vec<Assist>) =
                    std::mem::take(&mut c.engagement.assist)
                        .into_iter()
                        .partition(|a| a.due <= now);
                c.engagement.assist = wait;
                ready.into_iter().map(move |a| (g, a))
            })
            .collect();
        map.rare.assist = any;
    } else {
        debug_assert!(
            map.creatures
                .iter()
                .all(|(_, c)| c.engagement.assist.is_empty()),
            "a creature calls for help unnoted (RareStates)"
        );
    }
    for (caller, a) in due {
        // It must still be in the fight it called about.
        let still = map
            .creature(caller)
            .is_some_and(|c| c.alive() && c.combat.attacking && !c.evading);
        if !still {
            continue;
        }
        for h in a.helpers {
            if eligible(map, h, caller, a.enemy, false) {
                join(map, h, a.enemy);
            }
        }
    }
    let pulses: Vec<(Guid, Guid)> = map
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.alive()
                && c.combat.attacking
                && !c.evading
                && c.engagement.pulse_at.is_none_or(|t| t <= now)
                && dist(c.position, c.home) > PULSE_FROM_SPAWN
        })
        .map(|(_, c)| (c.guid, c.combat.victim))
        .collect();
    for (g, victim) in pulses {
        let range = call_range(map, g);
        if let Some(c) = map.creature_mut(g) {
            c.engagement.pulse_at = Some(now + PULSE_EVERY);
        }
        call_friends(map, g, victim, range);
    }
}

/// The nearest creature within 30 yards a fleeing `g` could run to for
/// help: of its faction template, answering calls, free, in sight.
pub fn nearest_helper(map: &mut Map, g: Guid) -> Option<Guid> {
    let enemy = map.creature(g)?.combat.victim;
    let at = map.creature(g)?.position;
    let mut near = around(map, g, SEEK_RADIUS, |m, h| eligible(m, h, g, enemy, false));
    near.sort_by(|a, b| {
        let d = |x: &Guid| map.creature(*x).map_or(f32::MAX, |c| dist(c.position, at));
        d(a).total_cmp(&d(b)).then(a.guid().cmp(&b.guid()))
    });
    near.first().copied()
}
