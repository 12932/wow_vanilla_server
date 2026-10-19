//! Scripted map events (vmangos's `ScriptedEvent`, script commands 61 to
//! 69): an event a script starts on a map, with its source and target,
//! extra targets, counters, a time limit, and success and failure
//! conditions; when one is met (or the time runs out) it ends and runs its
//! success or failure script, for itself and for each target.

use std::time::{Duration, Instant};

use ahash::AHashMap;
use wow_world_messages::Guid;

use crate::game::data::scripts::{ScriptStep, ScriptTable};
use crate::game::map::Map;

/// How often events are checked.
const CHECK_EVERY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Default)]
pub struct EventTarget {
    pub guid: Guid,
    pub success_condition: u32,
    pub failure_condition: u32,
    pub success_script: u32,
    pub failure_script: u32,
}

#[derive(Debug, Clone)]
pub struct ScriptedEvent {
    pub id: u32,
    pub source: Guid,
    pub target: Guid,
    pub expires: Instant,
    pub success_condition: u32,
    pub failure_condition: u32,
    pub success_script: u32,
    pub failure_script: u32,
    pub data: AHashMap<u32, u32>,
    pub targets: Vec<EventTarget>,
}

/// The map's events, and when they are next checked.
#[derive(Debug, Default)]
pub struct Events {
    pub active: AHashMap<u32, ScriptedEvent>,
    pub next_check: Option<Instant>,
}

pub fn active(map: &Map, id: u32) -> bool {
    map.map_events.active.contains_key(&id)
}

pub fn data(map: &Map, id: u32, index: u32) -> Option<u32> {
    map.map_events
        .active
        .get(&id)
        .map(|e| e.data.get(&index).copied().unwrap_or(0))
}

/// An event's source, target or (by entry) an extra target, for target
/// types 22, 23 and 24.
pub fn target_of(map: &Map, kind: u32, id: u32, entry: u32) -> Option<Guid> {
    let e = map.map_events.active.get(&id)?;
    match kind {
        22 => Some(e.source),
        23 => Some(e.target),
        _ => e
            .targets
            .iter()
            .find(|t| entry == 0 || map.creature(t.guid).is_some_and(|c| c.entry == entry))
            .map(|t| t.guid),
    }
}

/// The extra targets of event `id`.
pub fn targets(map: &Map, id: u32) -> Vec<Guid> {
    map.map_events
        .active
        .get(&id)
        .map(|e| e.targets.iter().map(|t| t.guid).collect())
        .unwrap_or_default()
}

fn int(v: i32) -> u32 {
    v.max(0) as u32
}

/// One of the map event commands, from `source` on `target`.
pub fn command(map: &mut Map, step: &ScriptStep, source: Guid, target: Guid) -> bool {
    let [id, d2, d3, d4] = step.datalong;
    let now = map.now;
    match step.command {
        61 => {
            if active(map, id) {
                return false;
            }
            map.map_events.active.insert(
                id,
                ScriptedEvent {
                    id,
                    source,
                    target,
                    expires: now + Duration::from_secs(u64::from(d2)),
                    success_condition: int(step.dataint[0]),
                    success_script: int(step.dataint[1]),
                    failure_condition: int(step.dataint[2]),
                    failure_script: int(step.dataint[3]),
                    data: AHashMap::new(),
                    targets: Vec::new(),
                },
            );
            map.map_events.next_check.get_or_insert(now + CHECK_EVERY);
            true
        }
        // An event not under way can't be ended (vmangos's "map event not
        // found", Thistle Tea's `:event_state`).
        62 => {
            if !active(map, id) {
                return false;
            }
            end(map, id, d2 != 0);
            true
        }
        63 => {
            let Some(e) = map.map_events.active.get_mut(&id) else {
                return false;
            };
            // The newest first, as Thistle Tea's list has them: an extra
            // target looked for is the newest of its entry.
            e.targets.retain(|t| t.guid != source);
            e.targets.insert(
                0,
                EventTarget {
                    guid: source,
                    success_condition: int(step.dataint[0]),
                    success_script: int(step.dataint[1]),
                    failure_condition: int(step.dataint[2]),
                    failure_script: int(step.dataint[3]),
                },
            );
            true
        }
        64 => {
            let Some(e) = map.map_events.active.get(&id).cloned() else {
                return false;
            };
            // By condition (1, 2) takes a condition.
            if matches!(d3, 1 | 2) && d2 == 0 {
                return false;
            }
            let met = |map: &Map, t: Guid| {
                crate::game::map::conditions::met(map, d2, Some(t), Some(source))
            };
            let kept: Vec<EventTarget> = match d3 {
                0 => e
                    .targets
                    .iter()
                    .filter(|t| t.guid != source)
                    .cloned()
                    .collect(),
                1 => {
                    let mut removed = false;
                    e.targets
                        .iter()
                        .filter(|t| {
                            if !removed && met(map, t.guid) {
                                removed = true;
                                false
                            } else {
                                true
                            }
                        })
                        .cloned()
                        .collect()
                }
                2 => e
                    .targets
                    .iter()
                    .filter(|t| !met(map, t.guid))
                    .cloned()
                    .collect(),
                3 => Vec::new(),
                _ => e.targets.clone(),
            };
            if let Some(e) = map.map_events.active.get_mut(&id) {
                e.targets = kept;
            }
            true
        }
        65 => {
            let Some(e) = map.map_events.active.get_mut(&id) else {
                return false;
            };
            let current = e.data.get(&d2).copied().unwrap_or(0);
            let value = match d4 {
                1 => current.saturating_add(d3),
                2 => current.saturating_sub(d3),
                _ => d3,
            };
            e.data.insert(d2, value);
            true
        }
        // Script event `id`, with data datalong2, for the event's source
        // and target (datalong3 0), its extra targets (1), or all of them.
        66 => {
            let Some(e) = map.map_events.active.get(&id).cloned() else {
                return false;
            };
            let main = [e.source, e.target];
            let extra = e.targets.iter().map(|t| t.guid);
            let mut recipients: Vec<Guid> = match d3 {
                0 => main.to_vec(),
                1 => extra.collect(),
                _ => main.into_iter().chain(extra).collect(),
            };
            let mut seen = ahash::AHashSet::new();
            recipients.retain(|g| crate::game::guid::is_unit(*g) && seen.insert(*g));
            for g in recipients {
                crate::game::map::event_ai::script_event(map, g, id, d2, None);
            }
            true
        }
        // Each new value (none below zero) replaces its old one; a zero
        // clears it.
        69 => {
            let Some(e) = map.map_events.active.get_mut(&id) else {
                return false;
            };
            let [success, success_script, failure, failure_script] = step.dataint;
            if success >= 0 {
                e.success_condition = int(success);
            }
            if success_script >= 0 {
                e.success_script = int(success_script);
            }
            if failure >= 0 {
                e.failure_condition = int(failure);
            }
            if failure_script >= 0 {
                e.failure_script = int(failure_script);
            }
            true
        }
        _ => false,
    }
}

/// Start event `id` from `source` on `target`, lasting `lasts`, with no
/// conditions or scripts of its own, if it isn't under way: whether it
/// started. An escort's event is its quest's, as Thistle Tea starts one
/// for each.
pub fn start(map: &mut Map, id: u32, source: Guid, target: Guid, lasts: Duration) -> bool {
    if active(map, id) {
        return false;
    }
    let now = map.now;
    map.map_events.active.insert(
        id,
        ScriptedEvent {
            id,
            source,
            target,
            expires: now + lasts,
            success_condition: 0,
            failure_condition: 0,
            success_script: 0,
            failure_script: 0,
            data: AHashMap::new(),
            targets: Vec::new(),
        },
    );
    map.map_events.next_check.get_or_insert(now + CHECK_EVERY);
    true
}

/// End event `id`: its success or failure scripts run, and an escort it
/// is the event of ends with it.
pub fn end(map: &mut Map, id: u32, success: bool) {
    let Some(e) = map.map_events.active.remove(&id) else {
        return;
    };
    crate::game::map::escorts::event_ended(map, id, e.source, success);
    let script = if success {
        e.success_script
    } else {
        e.failure_script
    };
    if script != 0 {
        crate::game::map::scripts::start(map, ScriptTable::Generic, script, e.source, e.target);
    }
    for t in &e.targets {
        let script = if success {
            t.success_script
        } else {
            t.failure_script
        };
        if script != 0 && map.position_of(t.guid).is_some() {
            crate::game::map::scripts::start(map, ScriptTable::Generic, script, t.guid, e.target);
        }
    }
}

fn met(map: &Map, condition: u32, source: Guid, target: Guid) -> bool {
    condition != 0 && crate::game::map::conditions::met(map, condition, Some(source), Some(target))
}

/// Check the events: run out, failed or succeeded.
pub fn update(map: &mut Map) {
    let now = map.now;
    if map.map_events.active.is_empty() || map.map_events.next_check.is_some_and(|t| t > now) {
        return;
    }
    map.map_events.next_check = Some(now + CHECK_EVERY);
    let events: Vec<ScriptedEvent> = map.map_events.active.values().cloned().collect();
    for e in events {
        // Out of time, or failed.
        let outcome = if now >= e.expires || met(map, e.failure_condition, e.source, e.target) {
            Some(false)
        } else if met(map, e.success_condition, e.source, e.target) {
            Some(true)
        } else {
            e.targets.iter().find_map(|t| {
                if map.position_of(t.guid).is_none() {
                    None
                } else if met(map, t.failure_condition, e.source, t.guid) {
                    Some(false)
                } else if met(map, t.success_condition, e.source, t.guid) {
                    Some(true)
                } else {
                    None
                }
            })
        };
        if let Some(success) = outcome {
            end(map, e.id, success);
        }
    }
}
