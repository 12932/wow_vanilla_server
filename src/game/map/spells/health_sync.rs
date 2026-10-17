//! Health through aura changes, as Thistle Tea's `HealthSync` keeps it: a
//! temporary health grant (Last Stand, Gift of Life) adds its health when
//! it comes and takes it when it goes, and a change to an ability's stamina
//! share or to bear form keeps the unit's share of its health.

use wow_world_messages::Guid;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, FORM_BEAR, FORM_DIREBEAR};

/// Temporary health grants: Last Stand's and Gift of Life's.
const TEMPORARY_HEALTH: [u32; 2] = [12976, 23782];

/// What a unit's health follows: its health and maximum, and what its
/// auras give it (as last seen, for the state before a change).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct State {
    health: u32,
    max: u32,
    temporary: i32,
    stamina: i32,
    bear: bool,
}

/// `g`'s health and maximum now, with what its auras gave when last seen
/// (nothing, for one not kept).
pub fn state(map: &Map, g: Guid) -> Option<State> {
    let f = map.unit_fields(g)?;
    let (health, max) = (
        f.get_u32(UNIT_FIELD_HEALTH),
        f.get_u32(UNIT_FIELD_MAXHEALTH),
    );
    if health == 0 || max == 0 {
        return None;
    }
    let seen = map.spells.health_seen.get(&g).copied().unwrap_or_default();
    Some(State { health, max, ..seen })
}

/// What `g`'s auras give its health now.
fn current(map: &Map, g: Guid) -> Option<State> {
    let f = map.unit_fields(g)?;
    let (health, max) = (
        f.get_u32(UNIT_FIELD_HEALTH),
        f.get_u32(UNIT_FIELD_MAXHEALTH),
    );
    let data = &map.realm.data;
    let a = auras::auras(map, g)?;
    let temporary = a
        .effects()
        .filter(|(h, x)| {
            TEMPORARY_HEALTH.contains(&h.spell) && x.kind == auras::AURA_MOD_INCREASE_HEALTH
        })
        .map(|(h, x)| x.amount * h.stacks.max(1) as i32)
        .sum();
    let stamina = a
        .effects()
        .filter(|(h, x)| {
            x.kind == auras::AURA_MOD_TOTAL_STAT_PERCENTAGE
                && x.misc == 2
                && data.spells.get(h.spell).is_some_and(|s| s.ability())
        })
        .map(|(_, x)| x.amount)
        .sum();
    let bear = matches!(auras::form(map, g), FORM_BEAR | FORM_DIREBEAR);
    Some(State {
        health,
        max,
        temporary,
        stamina,
        bear,
    })
}

/// `g`'s auras changed from `before`: its health follows.
pub fn sync(map: &mut Map, g: Guid, before: State) {
    let Some(after) = current(map, g) else {
        return;
    };
    // Kept only while its auras give something.
    if after.temporary != 0 || after.stamina != 0 || after.bear {
        map.spells.health_seen.insert(g, after);
    } else {
        map.spells.health_seen.remove(&g);
    }
    let scaled = before.stamina != after.stamina || before.bear != after.bear;
    let delta = after.temporary - before.temporary;
    if !scaled && delta == 0 {
        return;
    }
    let mut health = i64::from(before.health) + i64::from(delta);
    if scaled && before.max > 0 {
        // Its share of the new maximum (a bear's rounded up).
        let (h, old, new) = (health.max(0), i64::from(before.max), i64::from(after.max));
        health = if before.bear != after.bear {
            (new * h + old - 1) / old
        } else {
            new * h / old
        };
    }
    let health = health.clamp(1, i64::from(after.max.max(1))) as u32;
    if let Some(f) = map.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_HEALTH, health);
    }
}
