//! Who sees what. A player sees an object within the visibility radius
//! (on a flight master's mount, further: `[visibility] flight_yards`),
//! measured on the ground, and forgets it past the radius plus a margin,
//! so an object at the edge doesn't flicker. Visibility is worked out for an object when it has
//! moved some yards since the last time, or has just come into the map.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::Visibility;
use crate::game::guid;
use crate::game::map::player::Player;
use crate::game::map::{Map, death};
use crate::game::update;

/// How far past the radius an object is kept before it is forgotten.
pub const MARGIN: f32 = 10.0;
/// How far an object moves before its visibility is worked out again.
pub const MOVE_THRESHOLD: f32 = 8.0;
/// How far a stealthed unit moves before who sees it is worked out again.
pub const STEALTH_THRESHOLD: f32 = 2.0;

/// How far a player sees on `map`: a battleground's own distance, a
/// dungeon's or raid's, else the continents'.
pub fn radius(map: &Map) -> f32 {
    if crate::game::battlegrounds::bg_of_map(map.key.map).is_some() {
        map.cfg.battleground_visibility_yards
    } else if map.key.instance != 0 {
        map.cfg.instance_visibility_yards
    } else {
        map.cfg.visibility_yards
    }
}

/// How far `p` sees: on a flight master's mount, `[visibility]
/// flight_yards` (if further than the map's), else the map's.
pub fn sight(map: &Map, p: &Player) -> f32 {
    let r = radius(map);
    if p.flight.is_some() {
        r.max(map.cfg.flight_visibility_yards)
    } else {
        r
    }
}

/// [`sight`], and how far past it what `p` knows is kept, squared.
fn sight2(map: &Map, p: &Player) -> (f32, f32) {
    let r = sight(map, p);
    (r * r, (r + MARGIN) * (r + MARGIN))
}

/// Distances on the ground: height doesn't count.
fn dist2(a: Vector3d, b: Vector3d) -> f32 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

/// Note that `object` moved to `position`: its visibility is worked out at
/// the next update if it went far enough. A flier goes further between
/// looks, in proportion to how much further it sees (32 yards at 400):
/// what comes into its sight is as late, for its distance, as for a
/// walker, and a flight across a continent takes a quarter of the looks
/// (each over a window sixteen times the size).
///
/// A stealthed unit is looked at every [`STEALTH_THRESHOLD`] yards: who
/// sees it turns on yards (Thistle Tea's stealthed walker, announced every
/// two).
pub fn moved(map: &mut Map, object: Guid, anchor: Vector3d, position: Vector3d) {
    let stealthed = || {
        crate::game::map::auras::auras(map, object)
            .is_some_and(|a| a.has(crate::game::map::auras::AURA_MOD_STEALTH))
    };
    let threshold = match map.players.get(&object) {
        Some(p) if p.flight.is_some() => MOVE_THRESHOLD * sight(map, p) / radius(map),
        _ if stealthed() => STEALTH_THRESHOLD,
        _ => MOVE_THRESHOLD,
    };
    if dist2(anchor, position) >= threshold * threshold {
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
    map.fresh.clear();
    if map.visibility_dirty.is_empty() {
        return;
    }
    let dirty: Vec<Guid> = map.visibility_dirty.drain().collect();
    let z = crate::zone!("objects refreshed");
    z.value(dirty.len() as u64);
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
    // Arrived or moved on: the navmesh and terrain about them, ahead.
    if guid::is_player(object) {
        crate::game::map::tiles::ahead(map, pos);
    }
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
    let keep = radius(map) + MARGIN;

    // The players near the object: should they know it? Each by how far it
    // sees; one further off that sees further (a flier) is told of it by
    // its own looks, which go as far as it sees.
    let nearby: Vec<Guid> = map.grid.around(pos.x, pos.y, keep).collect();
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
        let (r2, keep2) = sight2(map, p);
        let d2 = dist2(crate::game::map::cinematic::view_point(p), pos);
        let knows = p.known.contains(&object);
        let sees = death::can_see(map, g, object);
        if !knows && d2 <= r2 && sees {
            introduce(map, g, object);
        } else if knows && (d2 > keep2 || !sees) {
            part(map, g, object);
        }
    }
    // Observers that have gone out of the grid window, and past their
    // sight.
    let far: Vec<Guid> = observers_of(map, object)
        .into_iter()
        .filter(|o| {
            map.players.get(o).is_none_or(|p| {
                dist2(crate::game::map::cinematic::view_point(p), pos) > sight2(map, p).1
            })
        })
        .collect();
    for o in far {
        part(map, o, object);
    }

    // A player: what should they know?
    let Some(me) = map.players.get(&object) else {
        return;
    };
    // What the player knows is what is near where they see from, as far
    // as they see.
    let view = crate::game::map::cinematic::view_point(me);
    let (r2, keep2) = sight2(map, me);
    let own = sight(map, me) + MARGIN;
    let (pos, nearby) = if dist2(view, pos) > 0.01 || own > keep {
        (view, map.grid.around(view.x, view.y, own).collect())
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

/// `observer`'s client learns of `object`: a player with quests in their
/// log as their group's members see them, with the log's ids (mangos's
/// `UF_FLAG_GROUP_ONLY`), else as anyone does.
pub fn introduce(map: &mut Map, observer: Guid, object: Guid) {
    // One aboard a boat the observer doesn't know yet: the boat first, so
    // the create naming it doesn't come before it.
    if let Some(t) = map
        .players
        .get(&object)
        .and_then(|p| p.movement.flags.get_on_transport())
        .map(|t| t.transport.guid)
        && map.gameobject(t).is_some()
        && map
            .players
            .get(&observer)
            .is_some_and(|p| !p.known.contains(&t))
    {
        introduce(map, observer, t);
    }
    let block = match map.players.get(&object) {
        Some(p)
            if crate::game::fields::has_group_only(&p.fields)
                && crate::game::map::objects::same_group(map, observer, object) =>
        {
            Some(std::sync::Arc::from(update::create_block(
                &p.create(false),
                crate::game::fields::Visibility::Group,
            )))
        }
        // A creature tapped by another: its grey view in its create.
        _ if guid::is_unit(object) => crate::game::map::tap::create_for(map, observer, object)
            .or_else(|| map.public_create(object)),
        _ => map.public_create(object),
    };
    let Some(block) = block else {
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
    map.batches
        .entry(observer)
        .or_default()
        .push_for(object, &block);
    // What the map sends it from now on, about the object or not, follows
    // the create.
    map.hold(observer);
    if let Some(o) = map.observers_mut(object) {
        o.insert(observer);
    }
    if map.creature_by_guid.contains_key(&object) {
        map.awake.insert(object);
    }
    crate::game::map::conditions::locate(map, object, false);
    // A banner taken by attacking it starts each viewer afresh (Thistle
    // Tea's `reset_banner_interaction`), once they have it (the client is
    // held).
    if guid::is_game_object(object)
        && crate::game::map::objects::resets_on_sight(map, object)
        && let Some(p) = map.players.get(&observer)
    {
        p.outbound
            .send(&wow_world_messages::vanilla::SMSG_GAMEOBJECT_RESET_STATE { guid: object });
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
    // Aboard a transport on the map: its create first, in the same packet,
    // so the client puts the player on its deck (mangos's `SendInitSelf`);
    // the player's own says where on it.
    let aboard = map
        .players
        .get(&guid)
        .and_then(|p| p.movement.flags.get_on_transport())
        .map(|t| t.transport.guid)
        .filter(|t| map.gameobject(*t).is_some());
    let transport = aboard.and_then(|t| Some((t, map.public_create(t)?)));
    if let Some((t, _)) = transport
        && let Some(o) = map.observers_mut(t)
    {
        o.insert(guid);
    }
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    let mut batch = update::Batch::new();
    if let Some((t, block)) = &transport {
        batch.push_for(*t, block);
        p.known.insert(*t);
    }
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
