//! Who sees what. A player sees an object within the visibility radius,
//! and forgets it past the radius plus a margin, so an object at the edge
//! doesn't flicker. Visibility is worked out for an object when it has
//! moved some yards since the last time, or has just come into the map.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::Visibility;
use crate::game::guid;
use crate::game::map::{Map, death};
use crate::game::update;

/// How far past the radius an object is kept before it is forgotten.
pub const MARGIN: f32 = 10.0;
/// How far an object moves before its visibility is worked out again.
pub const MOVE_THRESHOLD: f32 = 8.0;

fn dist2(a: Vector3d, b: Vector3d) -> f32 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

/// Note that `object` moved to `position`: its visibility is worked out at
/// the next update if it went far enough.
pub fn moved(map: &mut Map, object: Guid, anchor: Vector3d, position: Vector3d) {
    if dist2(anchor, position) >= MOVE_THRESHOLD * MOVE_THRESHOLD {
        map.visibility_dirty.insert(object);
    }
}

/// Work out the visibility of everything marked since the last update.
/// A unit's stealth or invisibility changed: who sees it is worked out
/// again.
pub fn stealth_changed(map: &mut Map, g: Guid) {
    refresh(map, g);
}

pub fn refresh_dirty(map: &mut Map) {
    if map.visibility_dirty.is_empty() {
        return;
    }
    let dirty: Vec<Guid> = map.visibility_dirty.drain().collect();
    for g in dirty {
        refresh(map, g);
    }
}

/// Who should know `object` now, and, if it is a player, what it should
/// know.
pub fn refresh(map: &mut Map, object: Guid) {
    let Some(pos) = map.position_of(object) else {
        return;
    };
    // A boat or zeppelin: everyone in the map knows it.
    if map.globals.contains(&object) {
        let unaware: Vec<Guid> = map
            .players
            .values()
            .filter(|p| !p.awaiting_worldport && !p.known.contains(&object))
            .map(|p| p.guid)
            .collect();
        for g in unaware {
            introduce(map, g, object);
        }
        return;
    }
    if guid::is_player(object) {
        if let Some(p) = map.players.get_mut(&object) {
            if p.awaiting_worldport {
                return;
            }
            p.vis_anchor = pos;
        }
        crate::game::map::world::update_zone(map, object);
    } else if let Some(c) = map.creature_mut(object) {
        c.vis_anchor = pos;
    }
    crate::game::map::conditions::locate(map, object, true);
    let radius = map.realm.config.visibility_yards;
    let keep = radius + MARGIN;
    let (r2, keep2) = (radius * radius, keep * keep);

    let nearby: Vec<Guid> = map.grid.around(pos.x, pos.y, keep).collect();

    // The players near the object: should they know it?
    for &g in &nearby {
        if g == object || !guid::is_player(g) {
            continue;
        }
        let Some(p) = map.players.get(&g) else {
            continue;
        };
        if p.awaiting_worldport {
            continue;
        }
        let d2 = dist2(crate::game::map::cinematic::view_point(p), pos);
        let knows = p.known.contains(&object);
        let sees = death::can_see(map, g, object);
        if !knows && d2 <= r2 && sees {
            introduce(map, g, object);
        } else if knows && (d2 > keep2 || !sees) {
            part(map, g, object);
        }
    }
    // Observers that have gone out of the grid window altogether.
    let far: Vec<Guid> = observers_of(map, object)
        .into_iter()
        .filter(|o| {
            map.players
                .get(o)
                .is_none_or(|p| dist2(crate::game::map::cinematic::view_point(p), pos) > keep2)
        })
        .collect();
    for o in far {
        part(map, o, object);
    }

    // A player: what should they know?
    if !guid::is_player(object) {
        return;
    }
    // What the player knows is what is near where they see from.
    let view = map
        .players
        .get(&object)
        .map_or(pos, crate::game::map::cinematic::view_point);
    let (pos, nearby) = if dist2(view, pos) > 0.01 {
        (view, map.grid.around(view.x, view.y, keep).collect())
    } else {
        (pos, nearby)
    };
    for &g in &nearby {
        if g == object {
            continue;
        }
        let Some(other) = map.position_of(g) else {
            continue;
        };
        if guid::is_player(g) && map.players.get(&g).is_some_and(|o| o.awaiting_worldport) {
            continue;
        }
        let knows = map
            .players
            .get(&object)
            .is_some_and(|p| p.known.contains(&g));
        let d2 = dist2(other, pos);
        let sees = death::can_see(map, object, g);
        if !knows && d2 <= r2 && sees {
            introduce(map, object, g);
        } else if knows && !sees {
            part(map, object, g);
        }
    }
    let globals = map.globals.clone();
    for g in globals {
        if map
            .players
            .get(&object)
            .is_some_and(|p| !p.known.contains(&g))
        {
            introduce(map, object, g);
        }
    }
    let gone: Vec<Guid> = map
        .players
        .get(&object)
        .map(|p| {
            p.known
                .iter()
                .copied()
                .filter(|k| {
                    !map.globals.contains(k)
                        && map
                            .position_of(*k)
                            .is_none_or(|other| dist2(other, pos) > keep2)
                })
                .collect()
        })
        .unwrap_or_default();
    for g in gone {
        part(map, object, g);
    }
}

fn observers_of(map: &Map, object: Guid) -> Vec<Guid> {
    map.observers(object)
        .map(|o| o.iter().copied().collect())
        .unwrap_or_default()
}

/// `observer`'s client learns of `object`.
pub fn introduce(map: &mut Map, observer: Guid, object: Guid) {
    let Some(block) = map.public_create(object) else {
        return;
    };
    let Some(p) = map.players.get_mut(&observer) else {
        return;
    };
    if !p.known.insert(object) {
        return;
    }
    // Made afresh: as no quest has it, until the next look.
    p.quest_objects.remove(&object);
    map.batches.entry(observer).or_default().push(&block);
    if let Some(o) = map.observers_mut(object) {
        o.insert(observer);
    }
    crate::game::map::conditions::locate(map, object, false);
    // A banner taken by attacking it starts each viewer afresh (Thistle
    // Tea's `reset_banner_interaction`), once they have it.
    if guid::is_game_object(object)
        && crate::game::map::objects::resets_on_sight(map, object)
        && let Some(frame) =
            crate::game::conn::encode(&wow_world_messages::vanilla::SMSG_GAMEOBJECT_RESET_STATE {
                guid: object,
            })
    {
        map.late_frames.push((observer, frame));
    }
}

/// `observer`'s client forgets `object`.
pub fn part(map: &mut Map, observer: Guid, object: Guid) {
    if let Some(p) = map.players.get_mut(&observer)
        && p.known.remove(&object)
    {
        map.forget.entry(observer).or_default().push(object);
    }
    map.remove_observer(object, observer);
}

/// The create blocks a player gets for themselves and their items, sent at
/// once (at login, and after a worldport).
pub fn send_self(map: &mut Map, guid: Guid) {
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    let mut batch = update::Batch::new();
    for item in &p.items {
        batch.push(&update::create_block(&item.create(), Visibility::Owner));
    }
    batch.push(&update::create_block(&p.create(true), Visibility::Owner));
    for frame in batch.finish() {
        p.outbound.send_frame(frame);
    }
    // The client has every field now; nobody else knows the player yet.
    p.fields.clear_dirty();
    for item in &mut p.items {
        item.fields.clear_dirty();
    }
}
