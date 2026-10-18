//! A map's supervision of what runs in it (docs/RELIABILITY.md), as
//! Thistle Tea's supervisors watch a process per creature and per player.
//!
//! Each step of the update, and each creature's turn, runs under a guard:
//! a panic costs that step or that turn, not the update. A creature that
//! panics again is restarted from its spawn (fresh, as a supervisor starts
//! a child again), and one that keeps panicking is quarantined: despawned
//! for good, and logged. A player whose packets keep panicking is saved
//! and sent back to the character screen, to come back with a fresh
//! state. A map whose update keeps panicking is in a bad state, and is
//! restarted: its players saved and sent to the character screen, and the
//! map built again from the data.
//!
//! None of it costs anything while nothing panics: `catch_unwind` is free,
//! and the counts are kept only for those that have panicked.

use std::time::{Duration, Instant};

use ahash::AHashMap;
use tracing::error;
use wow_world_messages::Guid;

use crate::game::map::creature::Life;
use crate::game::map::{Map, MapStats};
use crate::game::supervisor::{catch, worth_logging};

/// When those who panic have had enough.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A creature's panics count within this window.
    pub creature_window: Duration,
    /// At this many, it is restarted from its spawn ...
    pub creature_restart: u32,
    /// ... and at this many, quarantined.
    pub creature_quarantine: u32,
    /// A player's panicking packets count within this window ...
    pub player_window: Duration,
    /// ... and at this many, they go back to the character screen.
    pub player_out: u32,
    /// The update's panics count within this window ...
    pub map_window: Duration,
    /// ... and at this many, the map is restarted.
    pub map_restart: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            creature_window: Duration::from_secs(300),
            creature_restart: 2,
            creature_quarantine: 4,
            player_window: Duration::from_secs(60),
            player_out: 10,
            // A step that panics every tick for ten seconds.
            map_window: Duration::from_secs(60),
            map_restart: 200,
        }
    }
}

/// Panics in a window.
#[derive(Debug, Clone, Copy)]
struct Strikes {
    since: Instant,
    count: u32,
}

impl Strikes {
    fn new(now: Instant) -> Self {
        Self {
            since: now,
            count: 0,
        }
    }

    /// One more at `now`: how many in the window.
    fn hit(&mut self, now: Instant, window: Duration) -> u32 {
        if now.saturating_duration_since(self.since) > window {
            *self = Self::new(now);
        }
        self.count += 1;
        self.count
    }
}

/// Who in the map has panicked lately.
#[derive(Debug, Default)]
pub struct Health {
    pub limits: Limits,
    /// Creatures by their spawn: a respawn is the same creature.
    spawns: AHashMap<u32, Strikes>,
    /// Creatures with no spawn (summons), and players.
    units: AHashMap<Guid, Strikes>,
    update: Option<Strikes>,
}

impl Health {
    fn strike<K: std::hash::Hash + Eq>(
        map: &mut AHashMap<K, Strikes>,
        key: K,
        now: Instant,
        window: Duration,
    ) -> u32 {
        // Those long quiet are forgotten, so this stays small.
        if map.len() >= 64 {
            map.retain(|_, s| now.saturating_duration_since(s.since) <= window);
        }
        map.entry(key)
            .or_insert_with(|| Strikes::new(now))
            .hit(now, window)
    }
}

impl Map {
    /// Run one step of the update; a panic costs the step, this once.
    pub fn step(&mut self, name: &'static str, f: impl FnOnce(&mut Map)) {
        #[cfg(all(feature = "world-bench", unix))]
        let probe = super::bench_probe::enabled().then(super::bench_probe::thread_cpu);
        let result = catch(|| {
            #[cfg(test)]
            if self.faults.any() {
                self.faults.check(name);
            }
            f(self)
        });
        if let Err(e) = result {
            update_panicked(self, name, &e);
        }
        #[cfg(all(feature = "world-bench", unix))]
        if let Some(cpu) = probe {
            super::bench_probe::step_done(
                name,
                super::bench_probe::thread_cpu().saturating_sub(cpu),
            );
        }
    }
}

fn update_panicked(map: &mut Map, step: &str, e: &str) {
    map.stats.panics += 1;
    let (now, window) = (map.now, map.health.limits.map_window);
    let n = map
        .health
        .update
        .get_or_insert_with(|| Strikes::new(now))
        .hit(now, window);
    if worth_logging(u64::from(n)) {
        error!(
            "{}: the update's {step} panicked: {e}; the rest of the update goes on ({n} in {} s)",
            map.key,
            window.as_secs()
        );
    }
}

/// Whether the update has panicked so often that the map is in a bad
/// state, to be restarted.
pub fn escalated(map: &Map) -> bool {
    map.health.update.is_some_and(|s| {
        s.count >= map.health.limits.map_restart
            && map.now.saturating_duration_since(s.since) <= map.health.limits.map_window
    })
}

/// A player's packet panicked (and was dropped). One whose packets keep
/// panicking is saved and goes back to the character screen.
pub fn packet_panicked(map: &mut Map, g: Guid) {
    let (now, limits) = (map.now, map.health.limits);
    let n = Health::strike(&mut map.health.units, g, now, limits.player_window);
    if n < limits.player_out {
        return;
    }
    map.health.units.remove(&g);
    let name = map.players.get(&g).map(|p| p.name.clone());
    error!(
        "{}: {}'s packets panicked {n} times in {} s; saved and back to the character screen",
        map.key,
        name.as_deref().unwrap_or("a player"),
        limits.player_window.as_secs()
    );
    send_to_screen(map, g);
}

/// Save a player and send them to the character screen; if that panics
/// (the state that panicked may be theirs), they go unsaved rather than
/// stay.
fn send_to_screen(map: &mut Map, g: Guid) {
    if catch(|| crate::game::map::login::complete_logout(map, g)).is_ok() {
        return;
    }
    error!(
        "{}: {g} couldn't be saved; they go as they were last saved",
        map.key
    );
    if let Some(p) = map.players.remove(&g) {
        // What `take_player` does once its own steps are past.
        map.grid.remove(g, p.cell);
        map.visibility_dirty.remove(&g);
        map.batches.remove(&g);
        map.forget.remove(&g);
        for o in &p.observers {
            if let Some(op) = map.players.get_mut(o) {
                op.known.remove(&g);
                map.forget.entry(*o).or_default().push(g);
            }
        }
        for k in &p.known {
            map.remove_observer(*k, g);
        }
        map.realm.registry.go_offline(g);
        let _ = p
            .online
            .session
            .send(crate::game::session::Control::LoggedOut);
    }
}

/// Run a creature's turn; a panic in it costs the turn. One that panics
/// again is restarted from its spawn, and one that keeps on is
/// quarantined.
pub fn creature_turn(map: &mut Map, g: Guid, turn: impl FnOnce(&mut Map)) {
    let result = catch(|| {
        #[cfg(test)]
        if map.faults.any() {
            map.faults.check(&format!("creature {g}"));
        }
        turn(map)
    });
    if let Err(e) = result {
        creature_panicked(map, g, &e);
    }
}

fn creature_panicked(map: &mut Map, g: Guid, e: &str) {
    map.stats.panics += 1;
    let (now, limits) = (map.now, map.health.limits);
    let (spawn, entry) = map
        .creature(g)
        .map_or((None, 0), |c| (c.spawn_guid, c.entry));
    let n = match spawn {
        Some(s) => Health::strike(&mut map.health.spawns, s, now, limits.creature_window),
        None => Health::strike(&mut map.health.units, g, now, limits.creature_window),
    };
    let name = map
        .realm
        .data
        .creature_template(entry)
        .map_or_else(|| format!("creature {entry}"), |t| t.name.clone());
    let what = format!("{}: {name} ({g}) panicked in its turn: {e}", map.key);
    if n < limits.creature_restart {
        error!("{what}; the turn is dropped");
    } else if n < limits.creature_quarantine {
        error!("{what}; {n} times: restarted from its spawn");
        put_down(map, g, Duration::ZERO);
    } else {
        error!("{what}; {n} times: quarantined, despawned for good");
        put_down(map, g, Duration::from_secs(10 * 365 * 86_400));
        map.stats.quarantined += 1;
    }
}

/// Take a creature out of the world as though it died (no loot, no
/// credit), to come back from its spawn after `back_in`; one with no spawn
/// (a summon) is gone. If even that panics, it is simply removed.
fn put_down(map: &mut Map, g: Guid, back_in: Duration) {
    let downed = catch(|| crate::game::map::ai::quiet_death(map, g)).is_ok();
    if downed {
        let back = map.now + back_in;
        if let Some(c) = map.creature_mut(g)
            && let Life::Dead { respawn_at, .. } = &mut c.life
        {
            *respawn_at = back;
        }
        return;
    }
    if catch(|| map.remove_creature(g)).is_err() {
        map.creature_by_guid.remove(&g);
    }
    map.dead_creatures.retain(|d| *d != g);
}

/// Build the map again, as a supervisor starts a child afresh: its players
/// saved and sent to the character screen (from where they come back into
/// the new map), everything else from the data. The handle stays, so
/// what was sent to the old map is handled by the new one.
pub fn restart(mut old: Map) -> Map {
    let key = old.key;
    let players: Vec<Guid> = old.players.keys().copied().collect();
    error!(
        "{key}'s update panicked {} times in {} s: restarting it; its {} players are saved and go back to the character screen",
        old.health.update.map_or(0, |s| s.count),
        old.health.limits.map_window.as_secs(),
        players.len()
    );
    for g in players {
        send_to_screen(&mut old, g);
    }
    let mut map = Map::new(key, old.realm.clone(), old.handle.clone());
    map.stats = MapStats {
        updates: old.stats.updates,
        events: old.stats.events,
        panics: old.stats.panics,
        quarantined: old.stats.quarantined,
        restarts: old.stats.restarts + 1,
        ..Default::default()
    };
    // The old map's parts go with it; a panic in one of their drops
    // doesn't stop the new one.
    let _ = catch(move || drop(old));
    if catch(|| crate::game::map::world::populate(&mut map)).is_err() {
        error!("{key}: populating it again panicked; it starts empty");
    }
    let _ = catch(|| crate::game::transports::restore(&mut map));
    map
}
