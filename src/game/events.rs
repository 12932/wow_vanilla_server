//! Seasonal events: which are on, and the creatures and objects that come
//! and go with them. A spawn tied to an event is there while it runs (or,
//! tied with a negative id, while it doesn't). Each map adds and removes
//! them as events start and end.

use ahash::AHashSet;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::map::Map;
use crate::game::map::creature::Creature;
use crate::game::map::gameobject::GameObject;

/// The events on at `now` (Unix seconds).
pub fn active(data: &GameData, now: i64) -> AHashSet<u32> {
    data.events
        .values()
        .filter(|e| e.active(now))
        .map(|e| e.id)
        .collect()
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
    let on = active(&realm.data, now);
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
                    if let Some(c) = Creature::from_spawn(&data, s, &mut rng) {
                        map.add_creature(c);
                    }
                }
                (false, Some(g)) => {
                    map.remove_creature(g);
                }
                _ => {}
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
