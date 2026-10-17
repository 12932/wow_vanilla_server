//! Seeing from somewhere else, as Thistle Tea's `ViewpointSync` and far
//! sight do it: Far Sight and Eagle Eye put a dynamic object at the
//! targeted place and look from it for the channel; Mind Vision (bind
//! sight) looks through its target's eyes while the aura lasts. The
//! player's `PLAYER_FARSIGHT` names what they look from, and what they know
//! is what is near it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::visibility::MOVE_THRESHOLD;

use super::Targets;
use super::dynamic::{self, KIND_FARSIGHT};

fn dist2(a: Vector3d, b: Vector3d) -> f32 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

/// `viewer` looks from `object` from now on.
pub fn grant(map: &mut Map, viewer: Guid, object: Guid) {
    let Some(at) = map.position_of(object) else {
        return;
    };
    let Some(p) = map.players.get_mut(&viewer) else {
        return;
    };
    p.fields.set_guid(PLAYER_FARSIGHT, object);
    p.misc.view = Some(at);
    map.spells.viewpoints.insert(viewer, object);
    map.visibility_dirty.insert(viewer);
}

/// `viewer` stops looking from `object`, if that is where they look from.
pub fn release(map: &mut Map, viewer: Guid, object: Guid) {
    if map.spells.viewpoints.get(&viewer) != Some(&object) {
        return;
    }
    map.spells.viewpoints.remove(&viewer);
    if let Some(p) = map.players.get_mut(&viewer) {
        p.fields.set_guid(PLAYER_FARSIGHT, Guid::zero());
        p.misc.view = None;
    }
    map.visibility_dirty.insert(viewer);
}

/// `ADD_FARSIGHT`: a far sight focus at the targeted place, looked from.
pub fn far_sight(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    t: &Targets,
    ms: Option<u32>,
) {
    if !map.players.contains_key(&caster) {
        return;
    }
    let at = t
        .dest
        .or_else(|| t.unit.and_then(|u| map.position_of(u)))
        .or_else(|| map.position_of(caster));
    let Some(at) = at else {
        return;
    };
    let radius = map.realm.data.spells.radius(s.effects[index].radius_index);
    dynamic::remove_spell(map, caster, s.id);
    let focus = dynamic::spawn(map, caster, s.id, KIND_FARSIGHT, at, radius, ms);
    grant(map, caster, focus);
}

/// Viewpoints follow what they look from: a unit that walks off moves its
/// viewer's eyes, and one that is gone releases them.
pub fn update(map: &mut Map) {
    if map.spells.viewpoints.is_empty() {
        return;
    }
    let pairs: Vec<(Guid, Guid)> = map
        .spells
        .viewpoints
        .iter()
        .map(|(v, o)| (*v, *o))
        .collect();
    for (viewer, object) in pairs {
        let Some(at) = map.position_of(object) else {
            release(map, viewer, object);
            continue;
        };
        let Some(p) = map.players.get_mut(&viewer) else {
            map.spells.viewpoints.remove(&viewer);
            continue;
        };
        if p.misc
            .view
            .is_none_or(|v| dist2(v, at) >= MOVE_THRESHOLD * MOVE_THRESHOLD)
        {
            p.misc.view = Some(at);
            map.visibility_dirty.insert(viewer);
        }
    }
}
