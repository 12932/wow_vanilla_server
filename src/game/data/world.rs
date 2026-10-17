//! The world's data beyond the templates: points of interest, what events
//! change (quests offered, creatures' looks), escorts' paths, and the
//! moving objects' animations.

use ahash::AHashMap;
use rusqlite::Connection;

use super::load::rows;

/// `points_of_interest`: a spot a gossip option marks on the map.
#[derive(Debug, Clone, Default)]
pub struct Poi {
    pub x: f32,
    pub y: f32,
    pub icon: u32,
    pub flags: u32,
    pub data: u32,
    pub name: String,
}

/// `game_event_creature_data`: how a spawn looks while an event runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventCreature {
    pub event: u32,
    pub entry: u32,
    pub display: u32,
    pub equipment: u32,
    pub spell_start: u32,
    pub spell_end: u32,
}

/// `script_waypoint`: a point of an escort's path.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScriptPoint {
    pub point: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub wait_ms: u32,
}

/// `TransportAnimation`: where a moving object's model is at a time.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnimationFrame {
    pub time_ms: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Default)]
pub struct WorldData {
    pub pois: AHashMap<u32, Poi>,
    /// Quest → the event it is offered in.
    pub event_quests: AHashMap<u32, u32>,
    /// Spawn → its looks in each event.
    pub event_creatures: AHashMap<u32, Vec<EventCreature>>,
    pub script_waypoints: AHashMap<u32, Vec<ScriptPoint>>,
    /// Object entry → its animation's frames, in time order.
    pub animations: AHashMap<u32, Vec<AnimationFrame>>,
}

pub fn load(db: &Connection, dbc: Option<&Connection>, d: &mut WorldData) {
    for (entry, p) in rows(db, "SELECT * FROM points_of_interest", |c| {
        (
            c.u32("entry"),
            Poi {
                x: c.f32("x"),
                y: c.f32("y"),
                icon: c.u32("icon"),
                flags: c.u32("flags"),
                data: c.u32("data"),
                name: c.string("icon_name"),
            },
        )
    }) {
        d.pois.insert(entry, p);
    }
    for (quest, event) in rows(db, "SELECT * FROM game_event_quest WHERE patch_min <= 10", |c| {
        (c.u32("quest"), c.u32("event"))
    }) {
        d.event_quests.insert(quest, event);
    }
    for (guid, e) in rows(db, "SELECT * FROM game_event_creature_data WHERE patch <= 10", |c| {
        (
            c.u32("guid"),
            EventCreature {
                event: c.u32("event"),
                entry: c.u32("entry_id"),
                display: c.u32("display_id"),
                equipment: c.u32("equipment_id"),
                spell_start: c.u32("spell_start"),
                spell_end: c.u32("spell_end"),
            },
        )
    }) {
        let list = d.event_creatures.entry(guid).or_default();
        list.retain(|x| x.event != e.event);
        list.push(e);
    }
    for (entry, p) in rows(db, "SELECT * FROM script_waypoint ORDER BY entry, pointid", |c| {
        (
            c.u32("entry"),
            ScriptPoint {
                point: c.u32("pointid"),
                x: c.f32("location_x"),
                y: c.f32("location_y"),
                z: c.f32("location_z"),
                wait_ms: c.u32("waittime"),
            },
        )
    }) {
        d.script_waypoints.entry(entry).or_default().push(p);
    }
    let Some(dbc) = dbc else {
        return;
    };
    for (entry, f) in rows(
        dbc,
        "SELECT * FROM TransportAnimation ORDER BY transport, time_index",
        |c| {
            (
                c.u32("transport"),
                AnimationFrame {
                    time_ms: c.u32("time_index"),
                    x: c.f32("location_x"),
                    y: c.f32("location_y"),
                    z: c.f32("location_z"),
                },
            )
        },
    ) {
        d.animations.entry(entry).or_default().push(f);
    }
}
