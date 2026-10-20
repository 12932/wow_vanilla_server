//! Lady Jaina Proudmoore, the Alliance's champion: she comes from
//! Theramore to Stormwind, Ironforge and Darnassus as their alarms reach
//! the top, one Jaina for the three ([`crate::game::champions`]).
//!
//! Her fight is Thistle Tea's script of her, imported with the rest
//! (`data/tt_scripts.rs`); in a city her Teleport sends its victim to the
//! city's prison rather than to the top of her tower in Theramore
//! ([`crate::game::cities::Prison`]): the Stockade's door in Stormwind,
//! the Mystic Ward in Ironforge, the Temple of the Moon in Darnassus. Her
//! 40-yard rule is about that place.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::champions::Champion;
use crate::game::cities::Prison;
use crate::game::map::Map;

pub use crate::game::champions::{
    HOME_MAP, JAINA, JAINA_SPAWN as SPAWN, Place, State, TELEPORT_VISUAL, VANISH,
};

/// Her Teleport, to her tower's top (`spell_target_position`).
pub const TELEPORT: u32 = 20682;

/// Her copy in a city on this map, if she is there now.
pub fn copy(map: &Map) -> Option<Guid> {
    crate::game::champions::copy(map, Champion::Jaina)
}

/// Each update: see [`crate::game::champions::update`].
pub fn update(map: &mut Map) {
    crate::game::champions::update(map);
}

/// Where her Teleport sends its victim when she casts it in a city: the
/// city's prison.
pub fn teleport_to(map: &Map, caster: Guid, spell: u32) -> Option<(u32, Vector3d, f32)> {
    if spell != TELEPORT || map.creature(caster).is_none_or(|c| c.entry != JAINA) {
        return None;
    }
    let (copy, city) = crate::game::champions::away_here(map, Champion::Jaina)?;
    (copy == caster).then(|| city_spot(map, caster, city))
}

/// Her city's stand-in for her tower: its prison, else where she came.
fn city_spot(map: &Map, copy: Guid, city: u32) -> (u32, Vector3d, f32) {
    if let Some(p) = prison(map, city) {
        return p;
    }
    let (p, o) = map
        .creature(copy)
        .map_or((Vector3d::default(), 0.0), |c| (c.home, c.home_orientation));
    (map.key.map, p, o)
}

/// Where her Teleport sends its victim in `city`: its map, the spot and
/// the facing. An area trigger's spot is the data's, where it is on the
/// city's map.
pub fn prison(map: &Map, city: u32) -> Option<(u32, Vector3d, f32)> {
    let c = crate::game::cities::city(city)?;
    match c.prison.as_ref()? {
        Prison::Spot(p, o) => Some((c.map, *p, *o)),
        Prison::Trigger(id, p, o) => {
            let (p, o) = map
                .realm
                .data
                .area_triggers
                .get(id)
                .and_then(|t| t.teleport.as_ref())
                .filter(|t| t.map == c.map)
                .map_or((*p, *o), |t| {
                    (
                        Vector3d {
                            x: t.x,
                            y: t.y,
                            z: t.z,
                        },
                        t.orientation,
                    )
                });
            Some((c.map, p, o))
        }
    }
}

/// Just outside the Stockade's door, where its exit puts a player.
pub fn stockade_door(map: &Map) -> (Vector3d, f32) {
    prison(map, crate::game::cities::STORMWIND)
        .map_or(crate::game::cities::STOCKADE_DOOR, |p| (p.1, p.2))
}

/// A point a condition measures her from: her tower's top, in a city, is
/// the stand-in for it ([`teleport_to`]).
pub fn stand_in(map: &Map, who: Option<Guid>, point: [f32; 3]) -> [f32; 3] {
    let Some(who) = who.filter(|g| map.creature(*g).is_some_and(|c| c.entry == JAINA)) else {
        return point;
    };
    let tower = map.realm.data.spells.target_positions.get(&TELEPORT);
    let at_tower = tower.is_some_and(|&(_, x, y, z, _)| {
        (x - point[0]).abs() < 1.0 && (y - point[1]).abs() < 1.0 && (z - point[2]).abs() < 1.0
    });
    let away = crate::game::champions::away_here(map, Champion::Jaina);
    let Some((_, city)) = away.filter(|(g, _)| *g == who) else {
        return point;
    };
    if !at_tower {
        return point;
    }
    let (_, p, _) = city_spot(map, who, city);
    [p.x, p.y, p.z]
}
