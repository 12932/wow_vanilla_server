//! Indoors and outdoors, as Thistle Tea's `Spell.Environment` and
//! `SpellEnvironment` rule them: a spell only for outdoors (a mount, Travel
//! Form) can't be cast inside, one only for indoors can't be cast outside,
//! and an outdoor aura (and an outdoor passive) goes when its player walks
//! in, the passive coming back outside. The navmesh says which is which,
//! from the floor's WMO group.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;

use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::combat::dist;

/// How often a moving player's surroundings are looked at again.
const CHECK_EVERY: Duration = Duration::from_millis(1000);
const CHECK_YARDS: f32 = 2.0;

/// `SpellCastResult`s.
pub const ONLY_INDOORS: u8 = 0x52;
pub const ONLY_OUTDOORS: u8 = 0x55;

/// Whether `g` is outdoors; `None` where nothing says (no navmesh, or a
/// floor of unknown kind).
pub fn outdoors(map: &mut Map, g: Guid) -> Option<bool> {
    if let Some(v) = map.spells.outdoors_override.get(&g) {
        return Some(*v);
    }
    // On a boat or a zeppelin: only its open deck's spots.
    if let Some(t) = map
        .players
        .get(&g)
        .and_then(|p| p.movement.flags.get_on_transport())
    {
        let display = map
            .realm
            .data
            .gameobject_templates
            .get(&crate::game::guid::entry(t.transport.guid))
            .map_or(0, |o| o.display_id);
        return Some(on_open_deck(display, t.transport.position));
    }
    let at = map.position_of(g)?;
    map.nav.as_mut()?.outdoors(at)
}

/// Whether a passenger at `at` on a transport of `display` is under the
/// sky: a ship's (3015) three spots on deck, a zeppelin's (3031) one
/// (Thistle Tea's `Transport.outdoors?`).
pub fn on_open_deck(display: u32, at: wow_world_messages::vanilla::Vector3d) -> bool {
    let near = |(x, y, z, r): (f32, f32, f32, f32)| {
        ((at.x - x).powi(2) + (at.y - y).powi(2) + (at.z - z).powi(2)).sqrt() < r
    };
    match display {
        3015 => [
            (6.21, 0.12, 14.05, 1.1),
            (-10.46, 6.62, 17.77, 1.0),
            (1.55, -4.51, 11.30, 0.5),
        ]
        .into_iter()
        .any(near),
        3031 => near((-21.77, -7.90, -13.27, 1.1)),
        _ => false,
    }
}

/// Why a player can't cast `s` where they stand, if they can't.
pub fn check(map: &mut Map, caster: Guid, s: &SpellTemplate) -> Option<u8> {
    if !map.players.contains_key(&caster) {
        return None;
    }
    let indoors_only = s.attr(0, sd::ATTR_ONLY_INDOORS);
    let outdoors_only = s.attr(0, sd::ATTR_ONLY_OUTDOORS);
    if !indoors_only && !outdoors_only {
        return None;
    }
    match outdoors(map, caster) {
        Some(false) if outdoors_only => Some(ONLY_OUTDOORS),
        Some(true) if indoors_only => Some(ONLY_INDOORS),
        _ => None,
    }
}

/// A player moved: past a yard or two, or after a second, their outdoor
/// auras are reconciled with where they now are.
pub fn moved(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(at) = map.position_of(g) else {
        return;
    };
    if let Some((when, from)) = map.spells.outdoor_checked.get(&g)
        && now.duration_since(*when) < CHECK_EVERY
        && dist(*from, at) < CHECK_YARDS
    {
        return;
    }
    map.spells.outdoor_checked.insert(g, (now, at));
    reconcile(map, g, now);
    super::area::reconcile(map, g);
}

/// Outdoor auras go indoors; outdoor passives come back outdoors.
pub fn reconcile(map: &mut Map, g: Guid, _now: Instant) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if p.flight.is_some() || p.death != crate::game::map::death::Death::Alive {
        return;
    }
    let Some(outside) = outdoors(map, g) else {
        return;
    };
    let data = map.realm.data.clone();
    if !outside {
        let (mut gone, mut gear) = (false, false);
        for h in auras::auras(map, g).map_or(&[][..], |a| &a.holders[..]) {
            if data
                .spells
                .get(h.spell)
                .is_some_and(|s| s.attr(0, sd::ATTR_ONLY_OUTDOORS))
            {
                gone = true;
                gear |= h.equipped;
            }
        }
        if gone {
            auras::remove_where(map, g, |h| {
                data.spells
                    .get(h.spell)
                    .is_some_and(|s| s.attr(0, sd::ATTR_ONLY_OUTDOORS))
            });
        }
        // Gear's held off until outside again.
        if gear && let Some(p) = map.players.get_mut(&g) {
            p.gear_held_indoors = true;
        }
        return;
    }
    // Outside again: gear's outdoor spells held off come back.
    if let Some(p) = map.players.get_mut(&g)
        && std::mem::take(&mut p.gear_held_indoors)
    {
        p.gear_dirty = true;
    }
    // Outside again: the outdoor passives the player knows come back.
    let missing: Vec<u32> = map
        .players
        .get(&g)
        .map(|p| {
            p.spells
                .iter()
                .copied()
                .filter(|id| {
                    data.spells.get(*id).is_some_and(|s| {
                        s.passive() && s.attr(0, sd::ATTR_ONLY_OUTDOORS) && !p.auras.has_spell(*id)
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    for id in missing {
        super::apply_passive(map, g, id);
    }
}
