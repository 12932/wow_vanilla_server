//! The Scourge Invasion (vmangos's `ScourgeInvasionEvent`, hardcoded event
//! 17), as Thistle Tea's `GameEvent.ScourgeInvasion` and its system run it.
//! It starts off, as vmangos ships it; a GM turns it on (`.invasion on`).
//! While it runs, a pass every twenty seconds replays vmangos's
//! controller: a zone falls under attack once its rest is over (never the
//! one just won, and after the first victory only while no more than one
//! other is), its event brings the necropolises and camps, and a Mouth of
//! Kel'Thuzad stands over it; when every necropolis over a zone has fallen
//! the victory counts and the zone rests 45 to 60 minutes. Fifty, a hundred
//! and a hundred and fifty victories open the Argent Dawn's milestones; the
//! last ends the invasion. The tallies go out as world states.
//!
//! A camp's Necrotic Shard, destroyed, gives way to a damaged one; the
//! damaged one destroyed loses the camp and sends a death bolt to the
//! necropolis above, whose health dies to the third.

use ahash::{AHashMap, AHashSet};
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::Realm;
use crate::game::map::Map;
use crate::game::map::MapKey;

pub const INVASION: u32 = 17;
const FIFTY_VICTORIES: u32 = 96;
const HUNDRED_VICTORIES: u32 = 97;
const HUNDRED_FIFTY_VICTORIES: u32 = 98;
const INVASIONS_DONE: u32 = 99;
const BATTLES_WON_STATE: u32 = 2219;
const LAST_VICTORY: u32 = 150;
const FEWEST_ATTACK_SECS: i64 = 2_700;
const MOST_ATTACK_SECS: i64 = 3_600;
/// How often the controller looks the zones over.
pub const PASS_SECS: i64 = 20;
/// The Mouth of Kel'Thuzad, over an attacked zone.
pub const MOUTH: u32 = 16_995;
pub const NECROTIC_SHARD: u32 = 16_136;
pub const DAMAGED_SHARD: u32 = 16_172;
pub const NECROPOLIS_HEALTH: u32 = 16_421;
/// Death bolts a necropolis takes before it falls.
const BOLTS_TO_FALL: u32 = 3;

/// A zone the Scourge attacks.
#[derive(Debug, Clone, Copy)]
pub struct Zone {
    pub zone: u32,
    pub map: u32,
    pub event: u32,
    pub necropolises: u32,
    pub mouth: (f32, f32, f32, f32),
    pub invaded_state: u32,
    pub remaining_state: u32,
}

pub const ZONES: [Zone; 6] = [
    Zone {
        zone: 618,
        map: 1,
        event: 90,
        necropolises: 3,
        mouth: (7736.56, -4033.75, 696.327, 5.51524),
        invaded_state: 2259,
        remaining_state: 2284,
    },
    Zone {
        zone: 440,
        map: 1,
        event: 91,
        necropolises: 3,
        mouth: (-8352.68, -3972.68, 10.0753, 2.14675),
        invaded_state: 2263,
        remaining_state: 2283,
    },
    Zone {
        zone: 16,
        map: 1,
        event: 92,
        necropolises: 2,
        mouth: (3273.75, -4276.98, 125.509, 5.44543),
        invaded_state: 2260,
        remaining_state: 2279,
    },
    Zone {
        zone: 4,
        map: 0,
        event: 93,
        necropolises: 2,
        mouth: (-11_429.3, -3327.82, 7.73628, 1.0821),
        invaded_state: 2261,
        remaining_state: 2280,
    },
    Zone {
        zone: 139,
        map: 0,
        event: 94,
        necropolises: 2,
        mouth: (2014.55, -4934.52, 73.9846, 0.0698132),
        invaded_state: 2264,
        remaining_state: 2282,
    },
    Zone {
        zone: 46,
        map: 0,
        event: 95,
        necropolises: 2,
        mouth: (-8229.53, -1118.11, 144.012, 6.17846),
        invaded_state: 2262,
        remaining_state: 2281,
    },
];

/// The invasion's clock and tallies: vmangos's server variables.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub enabled: bool,
    pub victories: u32,
    pub last_zone: u32,
    /// When each zone may next be attacked.
    pub attack_at: AHashMap<u32, i64>,
    /// Necropolises still standing over each attacked zone.
    pub remaining: AHashMap<u32, u32>,
    /// Death bolts each zone's standing necropolis has taken.
    pub bolts: AHashMap<u32, u32>,
    pub active: AHashSet<u32>,
    pub next_pass: i64,
    /// The Mouth over each attacked zone.
    pub mouths: AHashMap<u32, Guid>,
    /// `[events] scourge_invasion` as last put in: the invasion follows the
    /// config when it changes, and a GM's `.invasion` in between.
    pub configured: Option<bool>,
}

impl State {
    fn may_attack(&self, z: &Zone, now: i64) -> bool {
        now >= self.attack_at.get(&z.zone).copied().unwrap_or(0)
            && z.zone != self.last_zone
            && !(self.active.len() > 1 && self.victories > 0)
    }
}

fn now() -> i64 {
    crate::game::social::mail::now()
}

/// The events the invasion holds on, as its tallies have them.
pub fn driven(s: &State) -> Vec<(u32, bool)> {
    let invading = s.enabled && s.victories < LAST_VICTORY;
    let done = s.enabled && s.victories >= LAST_VICTORY;
    let mut out: Vec<(u32, bool)> = ZONES
        .iter()
        .map(|z| (z.event, invading && s.active.contains(&z.zone)))
        .collect();
    out.extend([
        (INVASION, invading),
        (
            FIFTY_VICTORIES,
            invading && s.victories >= 50 && s.victories < 100,
        ),
        (HUNDRED_VICTORIES, invading && s.victories >= 100),
        (HUNDRED_FIFTY_VICTORIES, done),
        (INVASIONS_DONE, done),
    ]);
    out
}

/// The invasion's world states: victories, and each zone's necropolises.
pub fn world_states(s: &State) -> Vec<(u32, u32)> {
    let mut out = vec![(BATTLES_WON_STATE, s.victories)];
    for z in &ZONES {
        let left = s.remaining.get(&z.zone).copied().unwrap_or(0);
        out.push((z.invaded_state, left.min(1)));
        out.push((z.remaining_state, left));
    }
    out
}

fn publish(realm: &Realm) {
    let (events, states, running) = {
        let s = realm.social.invasion.lock();
        (driven(&s), world_states(&s), s.enabled)
    };
    for &(state, value) in &states {
        crate::game::map::world_states::set_global(realm, state, value);
    }
    // Called off, its states are told gone (0) and then left out of what
    // those coming in are told: shown only while it runs (Thistle Tea's
    // `world_states`).
    if !running {
        let mut w = realm.social.world_states.lock();
        for (state, _) in &states {
            w.remove(state);
        }
    }
    {
        let mut d = realm.social.driven_events.lock();
        for (e, on) in events {
            d.insert(e, on);
        }
    }
    crate::game::events::sweep(realm);
}

/// `.invasion on`: every zone that may be attacked falls at once.
pub fn enable(realm: &Realm) {
    let started = {
        let mut s = realm.social.invasion.lock();
        if s.enabled {
            return;
        }
        s.enabled = true;
        let t = now();
        let mut started = Vec::new();
        for z in &ZONES {
            if !s.active.contains(&z.zone) && s.may_attack(z, t) {
                s.remaining.insert(z.zone, z.necropolises);
                s.active.insert(z.zone);
                started.push(*z);
            }
        }
        s.next_pass = t + PASS_SECS;
        started
    };
    for z in started {
        mouth(realm, z, true);
    }
    publish(realm);
}

/// `.invasion off`: the Scourge are called off.
pub fn disable(realm: &Realm) {
    let gone: Vec<Zone> = {
        let mut s = realm.social.invasion.lock();
        s.enabled = false;
        let gone = ZONES
            .iter()
            .filter(|z| s.active.contains(&z.zone))
            .copied()
            .collect();
        s.active.clear();
        s.remaining.clear();
        s.bolts.clear();
        gone
    };
    for z in gone {
        mouth(realm, z, false);
    }
    publish(realm);
}

/// One pass of vmangos's controller over the zones, in order; first, the
/// invasion on or off as `[events] scourge_invasion` has newly put it.
pub fn sweep(realm: &Realm) {
    let want = realm.config().events.scourge_invasion;
    let turn = {
        let mut s = realm.social.invasion.lock();
        // Off at start, as the invasion is: the config's first word turns
        // it on only.
        let changed = s.configured.unwrap_or(false) != want;
        s.configured = Some(want);
        changed && s.enabled != want
    };
    if turn {
        if want {
            enable(realm);
        } else {
            disable(realm);
        }
    }
    let t = now();
    let (started, stopped) = {
        let mut s = realm.social.invasion.lock();
        if !s.enabled || t < s.next_pass {
            return;
        }
        s.next_pass = t + PASS_SECS;
        let mut rng = rand::rng();
        let mut started = Vec::new();
        let mut stopped = Vec::new();
        // Won, the invasion is over: the Scourge leave every zone and none
        // falls again (Thistle Tea's `over?` and `reset`).
        let over = s.victories >= LAST_VICTORY;
        if over {
            for z in &ZONES {
                if s.active.remove(&z.zone) {
                    stopped.push(*z);
                }
                s.attack_at.insert(z.zone, t);
            }
            s.remaining.clear();
            s.bolts.clear();
        }
        for z in ZONES.iter().filter(|_| !over) {
            let attack_at = t + rng.random_range(FEWEST_ATTACK_SECS..=MOST_ATTACK_SECS);
            if !s.active.contains(&z.zone) {
                if s.attack_at.get(&z.zone).copied().unwrap_or(0) < t && s.active.len() > 1 {
                    s.attack_at.insert(z.zone, attack_at);
                }
                if s.may_attack(z, t) {
                    s.remaining.insert(z.zone, z.necropolises);
                    s.active.insert(z.zone);
                    started.push(*z);
                }
            } else if s.attack_at.get(&z.zone).copied().unwrap_or(0) < t
                && s.remaining.get(&z.zone).copied().unwrap_or(0) == 0
            {
                s.attack_at.insert(z.zone, attack_at);
                s.victories += 1;
                s.last_zone = z.zone;
                s.active.remove(&z.zone);
                stopped.push(*z);
            }
        }
        (started, stopped)
    };
    for z in &started {
        mouth(realm, *z, true);
    }
    for z in &stopped {
        mouth(realm, *z, false);
    }
    if !started.is_empty() || !stopped.is_empty() {
        publish(realm);
    }
}

/// A necropolis over `zone` has fallen.
pub fn necropolis_fell(realm: &Realm, zone: u32) {
    {
        let mut s = realm.social.invasion.lock();
        let Some(left) = s.remaining.get_mut(&zone) else {
            return;
        };
        *left = left.saturating_sub(1);
        s.bolts.insert(zone, 0);
    }
    publish(realm);
}

/// The Mouth of Kel'Thuzad comes to or leaves an attacked zone.
fn mouth(realm: &Realm, z: Zone, arrive: bool) {
    // The continent is loaded once anyone is on it.
    let Some(handle) = realm
        .maps
        .all()
        .into_iter()
        .find(|h| h.key == MapKey::world(z.map))
    else {
        return;
    };
    let zone = z.zone;
    handle.call(move |m| {
        if arrive {
            let data = m.realm.data.clone();
            let counter = m.creature_counter();
            let spawn = crate::game::data::CreatureSpawn {
                guid: counter,
                entries: vec![MOUTH],
                map: z.map,
                x: z.mouth.0,
                y: z.mouth.1,
                z: z.mouth.2,
                orientation: z.mouth.3,
                ..Default::default()
            };
            if let Some(mut c) =
                crate::game::map::creature::Creature::from_spawn(&data, &spawn, &mut rand::rng())
            {
                c.spawn_guid = None;
                let g = m.add_creature(c);
                m.realm.social.invasion.lock().mouths.insert(zone, g);
            }
        } else {
            let g = m.realm.social.invasion.lock().mouths.remove(&zone);
            if let Some(g) = g {
                m.remove_creature(g);
            }
        }
    });
}

/// A creature died on `map` at `at`: a camp's shard gives way, a camp is
/// lost and bolts its necropolis, a necropolis falls.
pub fn creature_died(map: &mut Map, entry: u32, at: Vector3d) {
    if !matches!(entry, NECROTIC_SHARD | DAMAGED_SHARD | NECROPOLIS_HEALTH) {
        return;
    }
    let realm = map.realm.clone();
    if !realm.social.invasion.lock().enabled {
        return;
    }
    let zone = crate::game::map::world::zone_and_area(map, at)
        .map(|(z, _)| z)
        .or_else(|| {
            // Without a navmesh or terrain, the attacked zone of this map nearest.
            ZONES
                .iter()
                .filter(|z| {
                    z.map == map.key.map && realm.social.invasion.lock().active.contains(&z.zone)
                })
                .min_by(|a, b| {
                    let d = |z: &Zone| (z.mouth.0 - at.x).powi(2) + (z.mouth.1 - at.y).powi(2);
                    d(a).total_cmp(&d(b))
                })
                .map(|z| z.zone)
        })
        .unwrap_or(0);
    match entry {
        NECROTIC_SHARD => {
            let data = realm.data.clone();
            let counter = map.creature_counter();
            let spawn = crate::game::data::CreatureSpawn {
                guid: counter,
                entries: vec![DAMAGED_SHARD],
                map: map.key.map,
                x: at.x,
                y: at.y,
                z: at.z,
                ..Default::default()
            };
            if let Some(mut c) =
                crate::game::map::creature::Creature::from_spawn(&data, &spawn, &mut rand::rng())
            {
                c.spawn_guid = None;
                map.add_creature(c);
            }
        }
        DAMAGED_SHARD => {
            let fell = {
                let mut s = realm.social.invasion.lock();
                let bolts = s.bolts.entry(zone).or_insert(0);
                *bolts += 1;
                *bolts >= BOLTS_TO_FALL
            };
            if fell {
                necropolis_fell(&realm, zone);
            }
        }
        _ => necropolis_fell(&realm, zone),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_wave_takes_every_zone_and_later_ones_no_more_than_two() {
        let mut s = State {
            enabled: true,
            ..Default::default()
        };
        for z in &ZONES {
            assert!(s.may_attack(z, 0));
        }
        s.active.insert(618);
        s.active.insert(440);
        s.victories = 1;
        assert!(!s.may_attack(&ZONES[2], 0), "two attacked after a win");
        s.active.remove(&440);
        s.last_zone = 16;
        assert!(!s.may_attack(&ZONES[2], 0), "never the zone just won");
        assert!(s.may_attack(&ZONES[3], 0));
        let events = driven(&s);
        assert!(events.contains(&(INVASION, true)));
        assert!(events.contains(&(90, true)));
        assert!(events.contains(&(91, false)));
        s.remaining.insert(618, 2);
        let states = world_states(&s);
        assert!(states.contains(&(2259, 1)) && states.contains(&(2284, 2)));
    }
}
