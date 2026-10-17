//! Seasonal events: which are on, and the creatures and objects that come
//! and go with them. A spawn tied to an event is there while it runs (or,
//! tied with a negative id, while it doesn't); a spawn with event looks
//! wears them while its event runs. Each map adds and removes them as
//! events start and end. Quests tied to an event are offered only during it.
//!
//! Most events follow their recurrence; vmangos's hardcoded ones follow
//! the rules Thistle Tea ports (`GameEvent.Rule`): the Darkmoon Faire's
//! calendar, the fireworks of the holidays' nights, events always on in
//! this patch, and events the world drives (the invasions, Omen's minions)
//! through [`drive`].

use ahash::{AHashMap, AHashSet};
use chrono::{Datelike, Timelike};

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::map::Map;
use crate::game::map::creature::Creature;
use crate::game::map::gameobject::GameObject;

/// The events on at `now` (Unix seconds), with none driven.
pub fn active(data: &GameData, now: i64) -> AHashSet<u32> {
    active_with(data, now, &AHashMap::new())
}

/// The events on at `now`: those their recurrence has on, the hardcoded
/// ones their rules have on, and those the world drives.
pub fn active_with(data: &GameData, now: i64, driven: &AHashMap<u32, bool>) -> AHashSet<u32> {
    let mut on: AHashSet<u32> = data
        .events
        .values()
        .filter(|e| e.active(now))
        .map(|e| e.id)
        .collect();
    for id in rules(now, &on) {
        if data.events.get(&id).is_some_and(|e| e.hardcoded) {
            on.insert(id);
        }
    }
    for (id, driven_on) in driven {
        if *driven_on && data.events.contains_key(id) {
            on.insert(*id);
        }
    }
    on
}

/// The Darkmoon Faire's events: in Elwynn, in Mulgore, and being built in
/// each.
pub const DARKMOON_ELWYNN: u32 = 4;
pub const DARKMOON_MULGORE: u32 = 5;
pub const DARKMOON_BUILDING_ELWYNN: u32 = 23;
pub const DARKMOON_BUILDING_MULGORE: u32 = 24;
pub const FIREWORKS: u32 = 6;
pub const TOASTING_GOBLETS: u32 = 39;
pub const DRAGONS_OF_NIGHTMARE: u32 = 66;
pub const WAR_EFFORT_POST_WAR: u32 = 86;

/// The Darkmoon Faire on a day: it opens the first Monday of the month for
/// a week, in Elwynn in even months and Mulgore in odd ones, after three
/// days of building on the site (vmangos's `DarkmoonFaire`).
pub fn darkmoon(date: chrono::NaiveDate) -> Option<u32> {
    let first = date.with_day(1)?;
    let first_weekday = first.weekday().number_from_monday() as i32;
    let opening = (8 - first_weekday).rem_euclid(7) + 1;
    let day = date.day() as i32;
    let elwynn = date.month() % 2 == 0;
    if day + 3 < opening {
        None
    } else if day < opening {
        Some(if elwynn {
            DARKMOON_BUILDING_ELWYNN
        } else {
            DARKMOON_BUILDING_MULGORE
        })
    } else if day < opening + 7 {
        Some(if elwynn {
            DARKMOON_ELWYNN
        } else {
            DARKMOON_MULGORE
        })
    } else {
        None
    }
}

/// The hardcoded events' rules at `now`, given the events scheduled then.
pub fn rules(now: i64, scheduled: &AHashSet<u32>) -> Vec<u32> {
    let Some(t) = chrono::DateTime::from_timestamp(now, 0) else {
        return Vec::new();
    };
    let mut on = vec![DRAGONS_OF_NIGHTMARE, WAR_EFFORT_POST_WAR];
    on.extend(darkmoon(t.date_naive()));
    // The holidays' fireworks over the cities, the first ten minutes of
    // each night hour; goblets for the toasts after (`FireworksShow`).
    let night = t.hour() <= 6 || t.hour() >= 18;
    let any = |hs: &[u32]| hs.iter().any(|h| scheduled.contains(h));
    if night && t.minute() < 10 && any(&[34, 38, 41, 42]) {
        on.push(FIREWORKS);
    }
    if night && (10..=20).contains(&t.minute()) && any(&[34, 38]) {
        on.push(TOASTING_GOBLETS);
    }
    on
}

/// The world starts or stops an event it drives.
pub fn drive(realm: &Realm, event: u32, on: bool) {
    realm.social.driven_events.lock().insert(event, on);
    sweep(realm);
}

/// Whether a quest tied to an event may be offered now.
pub fn quest_offered(realm: &Realm, quest: u32) -> bool {
    match realm.data.world.event_quests.get(&quest) {
        Some(event) => realm.social.events.lock().contains(event),
        None => true,
    }
}

/// Whether a spawn tied (or not) to an event is out with `active` on.
pub fn spawned(tie: Option<i32>, active: &AHashSet<u32>) -> bool {
    match tie {
        None | Some(0) => true,
        Some(e) if e > 0 => active.contains(&(e as u32)),
        Some(e) => !active.contains(&e.unsigned_abs()),
    }
}

/// The events on now, as the realm last worked them out.
pub fn current(realm: &Realm) -> AHashSet<u32> {
    realm.social.events.lock().clone()
}

/// Events that started or ended: every map brings its spawns into line.
pub fn sweep(realm: &Realm) {
    let now = crate::game::social::mail::now();
    let driven = realm.social.driven_events.lock().clone();
    let on = active_with(&realm.data, now, &driven);
    {
        let mut events = realm.social.events.lock();
        if *events == on {
            return;
        }
        for e in on.difference(&events) {
            if let Some(ev) = realm.data.events.get(e) {
                tracing::info!("event {} ({}) begins", e, ev.name);
            }
        }
        for e in events.difference(&on) {
            if let Some(ev) = realm.data.events.get(e) {
                tracing::info!("event {} ({}) ends", e, ev.name);
            }
        }
        *events = on.clone();
    }
    for h in realm.maps.all() {
        let on = on.clone();
        h.call(move |m| apply(m, &on));
    }
}

/// A spawn's looks in the events on: the lowest event's.
pub fn looks(
    data: &GameData,
    spawn: u32,
    on: &AHashSet<u32>,
) -> Option<crate::game::data::world::EventCreature> {
    let mut list = data.world.event_creatures.get(&spawn)?.clone();
    list.sort_by_key(|e| e.event);
    list.into_iter().find(|e| on.contains(&e.event))
}

/// A creature for a spawn, as it looks in the events on.
pub fn spawn_creature(
    data: &GameData,
    s: &crate::game::data::CreatureSpawn,
    on: &AHashSet<u32>,
    rng: &mut impl rand::Rng,
) -> Option<Creature> {
    let Some(e) = looks(data, s.guid, on) else {
        return Creature::from_spawn(data, s, rng);
    };
    let event = e.event;
    let mut spawn = s.clone();
    if e.entry != 0 {
        spawn.entries = vec![e.entry];
    }
    let mut c = Creature::from_spawn(data, &spawn, rng)?;
    if e.display != 0 {
        c.fields
            .set_u32(crate::game::fields::UNIT_FIELD_DISPLAYID, e.display);
        c.fields
            .set_u32(crate::game::fields::UNIT_FIELD_NATIVEDISPLAYID, e.display);
    }
    if e.equipment != 0
        && let Some(items) = data.creature_equipment.get(&e.equipment)
    {
        c.set_equipment(data, *items);
    }
    c.event_looks = event;
    Some(c)
}

/// A map's event spawns, in line with the events on.
pub fn apply(map: &mut Map, on: &AHashSet<u32>) {
    let data = map.realm.data.clone();
    let mut rng = rand::rng();
    if let Some(spawns) = data.creature_spawns.get(&map.key.map) {
        for s in spawns {
            let Some(tie) = data.event_creatures.get(&s.guid).copied() else {
                continue;
            };
            let out = map
                .creatures
                .iter()
                .find(|(_, c)| c.spawn_guid == Some(s.guid))
                .map(|(_, c)| c.guid);
            match (spawned(Some(tie), on), out) {
                (true, None) => {
                    if let Some(c) = spawn_creature(&data, s, on, &mut rng) {
                        map.add_creature(c);
                    }
                }
                (false, Some(g)) => {
                    map.remove_creature(g);
                }
                _ => {}
            }
        }
        // Spawns that look different in an event change as it starts and
        // ends.
        for s in spawns.iter().filter(|s| data.world.event_creatures.contains_key(&s.guid)) {
            let Some((g, worn)) = map
                .creatures
                .iter()
                .find(|(_, c)| c.spawn_guid == Some(s.guid))
                .map(|(_, c)| (c.guid, c.event_looks))
            else {
                continue;
            };
            let fits = worn == looks(&data, s.guid, on).map_or(0, |e| e.event);
            if !fits {
                map.remove_creature(g);
                if let Some(c) = spawn_creature(&data, s, on, &mut rng) {
                    map.add_creature(c);
                }
            }
        }
    }
    if let Some(spawns) = data.gameobject_spawns.get(&map.key.map) {
        for s in spawns {
            let Some(tie) = data.event_objects.get(&s.guid).copied() else {
                continue;
            };
            let out = map
                .gameobjects
                .iter()
                .find(|(_, g)| g.spawn_guid == Some(s.guid))
                .map(|(_, g)| g.guid);
            match (spawned(Some(tie), on), out) {
                (true, None) => {
                    if let Some(t) = data.gameobject_templates.get(&s.entry) {
                        map.add_gameobject(GameObject::from_spawn(t, s));
                    }
                }
                (false, Some(g)) => {
                    map.remove_gameobject(g);
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::data::GameEvent;

    #[test]
    fn an_event_runs_its_length_every_occurrence() {
        // From the start, for an hour of every day.
        let e = GameEvent {
            id: 1,
            name: "Hourly".into(),
            start: 1_000_000,
            end: 2_000_000,
            occurence: 1440,
            length: 60,
            hardcoded: false,
        };
        assert!(e.active(1_000_000));
        assert!(e.active(1_000_000 + 3599));
        assert!(!e.active(1_000_000 + 3600));
        assert!(e.active(1_000_000 + 86_400 + 10));
        assert!(!e.active(999_999));
        assert!(!e.active(2_000_000));
        let on: AHashSet<u32> = [1].into_iter().collect();
        assert!(spawned(Some(1), &on));
        assert!(!spawned(Some(-1), &on));
        assert!(spawned(Some(-2), &on));
        assert!(spawned(None, &on));
    }
}
