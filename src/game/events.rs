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
use wow_world_messages::Guid;

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
    let elwynn = date.month().is_multiple_of(2);
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

/// A GM starts or stops an event (mangos' `.event start|stop`, which
/// moves the event's start): started, it runs its length from now; stopped,
/// it is over until its next turn would come round. `None` if there is no
/// such event, `Some(false)` if it already was so.
pub fn force(realm: &Realm, event: u32, on: bool) -> Option<bool> {
    let e = realm.data.events.get(&event)?;
    if current(realm).contains(&event) == on {
        return Some(false);
    }
    let now = crate::game::social::mail::now();
    let until = if on && e.length > 0 {
        now + e.length * 60
    } else if !on && e.length > 0 && e.occurence > e.length {
        now + (e.occurence - e.length) * 60
    } else {
        i64::MAX
    };
    realm.social.gm_events.lock().insert(event, (on, until));
    sweep(realm);
    Some(true)
}

/// The GM's starts and stops still in force, over what is on.
fn overridden(realm: &Realm, now: i64, on: &mut AHashSet<u32>) {
    let mut forced = realm.social.gm_events.lock();
    forced.retain(|_, (_, until)| *until > now);
    for (e, (start, _)) in forced.iter() {
        if *start {
            on.insert(*e);
        } else {
            on.remove(e);
        }
    }
}

/// Whether a quest tied to an event may be offered now.
pub fn quest_offered(realm: &Realm, quest: u32) -> bool {
    match realm.data.world.event_quests.get(&quest) {
        Some(event) => realm.social.events.lock().contains(event),
        None => true,
    }
}

/// Whether a spawn tied (or not) to events is out with `active` on: in any
/// event it is tied to (a positive id), if any; in none it is kept out of
/// (a negative one), as vmangos spawns and unspawns each event's own list.
pub fn spawned(ties: &[i32], active: &AHashSet<u32>) -> bool {
    let mut during = false;
    let mut on = false;
    for &e in ties {
        if e > 0 {
            during = true;
            on |= active.contains(&e.unsigned_abs());
        } else if e < 0 && active.contains(&e.unsigned_abs()) {
            return false;
        }
    }
    !during || on
}

/// A spawn's ties to events, from `game_event_creature` or
/// `game_event_gameobject`.
pub fn ties(of: &AHashMap<u32, Vec<i32>>, spawn: u32) -> &[i32] {
    of.get(&spawn).map_or(&[], Vec::as_slice)
}

/// The events on now, as the realm last worked them out.
pub fn current(realm: &Realm) -> AHashSet<u32> {
    realm.social.events.lock().clone()
}

/// Events that started or ended: every map brings its spawns into line.
pub fn sweep(realm: &Realm) {
    let now = crate::game::social::mail::now();
    let driven = realm.social.driven_events.lock().clone();
    let mut on = active_with(&realm.data, now, &driven);
    overridden(realm, now, &mut on);
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
        // Its size and reach the model's (Thistle Tea's explicit event
        // model).
        c.fit_model(data, None);
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
    // What is out, by spawn: one pass, not one a spawn.
    let creatures: AHashMap<u32, (Guid, u32)> = map
        .creatures
        .iter()
        .filter_map(|(_, c)| c.spawn_guid.map(|s| (s, (c.guid, c.event_looks))))
        .collect();
    let objects: AHashMap<u32, Guid> = map
        .gameobjects
        .iter()
        .filter_map(|(_, g)| g.spawn_guid.map(|s| (s, g.guid)))
        .collect();
    if let Some(spawns) = data.creature_spawns.get(&map.key.map) {
        for s in spawns {
            let tie = data.event_creatures.get(&s.guid);
            let looks_differ = data.world.event_creatures.contains_key(&s.guid);
            if tie.is_none() && !looks_differ {
                continue;
            }
            let out = creatures.get(&s.guid).copied();
            // A pooled one comes and goes as its pool picks (below).
            let pooled = tie.is_some()
                && crate::game::map::pools::pooled(
                    map,
                    crate::game::map::pools::Member::Creature(s.guid),
                );
            match (spawned(ties(&data.event_creatures, s.guid), on), out) {
                (true, None) | (false, Some(_)) if pooled => {}
                (true, None) => {
                    if let Some(c) = spawn_creature(&data, s, on, &mut rng) {
                        let g = map.add_creature(c);
                        came_in(map, g);
                    }
                }
                (false, Some((g, _))) => {
                    map.remove_creature(g);
                }
                // Out, and looking as it should in the events on? A living
                // one changes where it stands, its fight and all (mangos's
                // `UpdateEntry` from `UpdateCreatureData`, Thistle Tea's
                // `CreatureEvent.reconcile`); a corpse takes them at its
                // respawn.
                (true, Some((g, worn)))
                    if looks_differ && worn != looks(&data, s.guid, on).map_or(0, |e| e.event) =>
                {
                    if map.creature(g).is_some_and(|c| c.alive() && !c.despawned) {
                        reconcile(map, g, s, on);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(spawns) = data.gameobject_spawns.get(&map.key.map) {
        for s in spawns {
            let Some(tie) = data.event_objects.get(&s.guid) else {
                continue;
            };
            let out = objects.get(&s.guid).copied();
            if crate::game::map::pools::pooled(map, crate::game::map::pools::Member::Object(s.guid))
            {
                continue;
            }
            match (spawned(tie, on), out) {
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
    // Pools whose spawns the events allow changed pick anew.
    use crate::game::map::pools::{self, Member};
    for m in pools::events_changed(map, on) {
        match m {
            Member::Creature(s) => crate::game::map::ai::spawn_creature_of(map, s),
            Member::Object(s) => pools::spawn_object(map, s),
            Member::Pool(_) => {}
        }
    }
}

/// The looks of `event` for `spawn`, if it has them.
fn looks_of(
    data: &GameData,
    spawn: u32,
    event: u32,
) -> Option<crate::game::data::world::EventCreature> {
    data.world
        .event_creatures
        .get(&spawn)?
        .iter()
        .find(|e| e.event == event)
        .copied()
}

/// The spells that go with an event's looks (mangos's
/// `ApplyGameEventSpells`): as they begin, the end's aura off and the
/// start's cast; as they end, the start's aura off and the end's cast, the
/// creature on itself.
fn event_spells(
    map: &mut Map,
    g: Guid,
    looks: &crate::game::data::world::EventCreature,
    begins: bool,
) {
    let (cast, remove) = if begins {
        (looks.spell_start, looks.spell_end)
    } else {
        (looks.spell_end, looks.spell_start)
    };
    if remove != 0 {
        crate::game::map::auras::remove_spell(map, g, remove, None);
    }
    if cast != 0 && map.realm.data.spells.get(cast).is_some() {
        crate::game::map::spells::cast_triggered(map, g, cast, Some(g));
    }
}

/// A spawn's creature come into the world in an event's looks, alive, takes
/// their start spell (mangos's `UpdateEntry` with its event's data).
pub fn came_in(map: &mut Map, g: Guid) {
    let Some(c) = map.creature(g).filter(|c| c.alive() && c.event_looks != 0) else {
        return;
    };
    let (spawn, worn) = (c.spawn_guid, c.event_looks);
    let Some(e) = spawn.and_then(|s| looks_of(&map.realm.data, s, worn)) else {
        return;
    };
    event_spells(map, g, &e, true);
}

/// A living creature takes the looks of the events on where it stands
/// (mangos's `UpdateEntry` from `GameEventMgr::UpdateCreatureData`, Thistle
/// Tea's `CreatureEvent.reconcile`): the entry they name, or else its
/// spawn's own, its fight, health in proportion and auras kept, and its
/// own spell list, its timers anew; its look, size and weapons as it would
/// spawn in them, a morph on it kept over its new look; and the spells
/// that go with the looks it leaves and takes.
fn reconcile(map: &mut Map, g: Guid, s: &crate::game::data::CreatureSpawn, on: &AHashSet<u32>) {
    use crate::game::fields::*;
    let data = map.realm.data.clone();
    let Some(c) = map.creature(g) else {
        return;
    };
    let old = looks_of(&data, s.guid, c.event_looks);
    let new = looks(&data, s.guid, on);
    let display = c.fields.get_u32(UNIT_FIELD_DISPLAYID);
    let morph = (display != c.fields.get_u32(UNIT_FIELD_NATIVEDISPLAYID)).then_some(display);
    // Its own entry: the one it is, unless that is the looks' it leaves.
    let own = if s.entries.contains(&c.entry) && old.is_none_or(|o| o.entry != c.entry) {
        c.entry
    } else {
        s.entries.first().copied().unwrap_or(c.entry)
    };
    let entry = new.map(|e| e.entry).filter(|e| *e != 0).unwrap_or(own);
    let mut spawn = s.clone();
    spawn.entries = vec![entry];
    let Some(fresh) = spawn_creature(&data, &spawn, on, &mut rand::rng()) else {
        return;
    };
    if let Some(o) = old {
        event_spells(map, g, &o, false);
    }
    if map.creature(g).is_some_and(|c| c.entry != entry) {
        crate::game::map::scripts::update_entry(map, g, entry);
        if let Some(c) = map.creature_mut(g) {
            c.caster.spell_list = None;
            if let Some(s) = c.scripted.as_deref_mut() {
                s.base_spell_list = None;
            }
        }
    }
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let f = &fresh.fields;
    c.fields.set_u32(
        UNIT_FIELD_NATIVEDISPLAYID,
        f.get_u32(UNIT_FIELD_NATIVEDISPLAYID),
    );
    c.fields.set_u32(
        UNIT_FIELD_DISPLAYID,
        morph.unwrap_or_else(|| f.get_u32(UNIT_FIELD_DISPLAYID)),
    );
    let grown = c.auras.product(crate::game::map::auras::AURA_MOD_SCALE);
    c.fields.set_f32(
        OBJECT_FIELD_SCALE_X,
        f.get_f32(OBJECT_FIELD_SCALE_X) * grown,
    );
    for field in [UNIT_FIELD_BOUNDINGRADIUS, UNIT_FIELD_COMBATREACH] {
        c.fields.set_f32(field, f.get_f32(field));
    }
    c.fields
        .set_byte(UNIT_FIELD_BYTES_0, 2, f.get_byte(UNIT_FIELD_BYTES_0, 2));
    for i in 0..3 {
        c.fields.set_u32(
            UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i,
            f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i),
        );
    }
    for i in 0..6 {
        c.fields.set_u32(
            UNIT_VIRTUAL_ITEM_INFO + i,
            f.get_u32(UNIT_VIRTUAL_ITEM_INFO + i),
        );
    }
    c.event_looks = fresh.event_looks;
    if let Some(n) = new {
        event_spells(map, g, &n, true);
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
        assert!(spawned(&[1], &on));
        assert!(!spawned(&[-1], &on));
        assert!(spawned(&[-2], &on));
        assert!(spawned(&[], &on));
    }
}
