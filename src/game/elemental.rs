//! The Elemental Invasion (vmangos's `ElementalInvasion`, hardcoded event
//! 13), as Thistle Tea's `GameEvent.ElementalInvasion` runs it. The world
//! starts invaded: rifts in Un'Goro, Silithus, Azshara and Winterspring
//! keep invaders about them, more as each element's stage climbs, a stage
//! for every fifty slain or every hour endured, until at the fifth the
//! element's lord walks out. A lord's death closes its rifts and, after five
//! minutes' looting, ends its event; when all four have fallen the
//! invasion rests two to four days and comes again.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::Realm;
use crate::game::map::Map;

pub const INVASION: u32 = 13;
const FIRST_STAGE: u32 = 1;
const BOSS_STAGE: u32 = 5;
const BOSS_DOWN: u32 = 6;
const KILLS_PER_STAGE: u32 = 50;
const HOUR: i64 = 3_600;
const LOOTING: i64 = 300;
const REST: std::ops::RangeInclusive<i64> = 2 * 86_400..=4 * 86_400;
/// How often a rift calls out invaders.
pub const UPKEEP: Duration = Duration::from_secs(70);
const INVADER_LIFETIME: Duration = Duration::from_secs(3_600);

/// An element: its events, rift, invader and lord.
#[derive(Debug, Clone, Copy)]
pub struct Element {
    pub rift_event: u32,
    pub boss_event: u32,
    pub rift: u32,
    pub invader: u32,
    pub lord: u32,
}

pub const ELEMENTS: [Element; 4] = [
    // Fire, in Un'Goro.
    Element {
        rift_event: 68,
        boss_event: 72,
        rift: 179_666,
        invader: 14_460,
        lord: 14_461,
    },
    // Air, in Silithus.
    Element {
        rift_event: 69,
        boss_event: 73,
        rift: 179_667,
        invader: 14_455,
        lord: 14_454,
    },
    // Earth, in Azshara.
    Element {
        rift_event: 70,
        boss_event: 74,
        rift: 179_664,
        invader: 14_462,
        lord: 14_464,
    },
    // Water, in Winterspring.
    Element {
        rift_event: 71,
        boss_event: 75,
        rift: 179_665,
        invader: 14_458,
        lord: 14_457,
    },
];

/// The invaders a rift keeps out at `stage`.
pub fn invaders(stage: u32) -> u32 {
    if (FIRST_STAGE..=BOSS_STAGE).contains(&stage) {
        (3 + stage - 1).min(6)
    } else {
        0
    }
}

#[derive(Debug, Clone, Default)]
pub struct State {
    pub started: bool,
    pub stages: [u32; 4],
    pub kills: [u32; 4],
    /// A fallen lord's event stays open while their corpse is looted.
    pub looting_until: [Option<i64>; 4],
    pub next_hour: i64,
    pub rest_until: Option<i64>,
}

impl State {
    fn begin(&mut self, now: i64) {
        self.stages = [FIRST_STAGE; 4];
        self.kills = [0; 4];
        self.looting_until = [None; 4];
        self.next_hour = now + HOUR;
        self.rest_until = None;
        self.started = true;
    }

    fn looting(&self, i: usize, now: i64) -> bool {
        self.looting_until[i].is_some_and(|t| t > now)
    }

    /// The events the invasion holds on.
    pub fn driven(&self, now: i64) -> Vec<(u32, bool)> {
        let invading =
            self.started && (0..4).any(|i| self.stages[i] < BOSS_DOWN || self.looting(i, now));
        let mut out = vec![(INVASION, invading)];
        for (i, e) in ELEMENTS.iter().enumerate() {
            let stage = self.stages[i];
            let boss = stage == BOSS_STAGE || (stage == BOSS_DOWN && self.looting(i, now));
            out.push((e.rift_event, invading && stage < BOSS_DOWN));
            out.push((e.boss_event, invading && boss));
        }
        out
    }

    /// An invader of element `i` slain.
    pub fn slain(&mut self, i: usize) {
        if (FIRST_STAGE..BOSS_STAGE).contains(&self.stages[i]) {
            self.kills[i] += 1;
            if self.kills[i] >= KILLS_PER_STAGE {
                self.stages[i] += 1;
                self.kills[i] = 0;
            }
        }
    }
}

fn now() -> i64 {
    crate::game::social::mail::now()
}

fn drive(realm: &Realm) {
    let events = realm.social.elemental.lock().driven(now());
    {
        let mut d = realm.social.driven_events.lock();
        for (e, on) in events {
            d.insert(e, on);
        }
    }
    crate::game::events::sweep(realm);
}

/// The invasion's clock: it begins, endures its hours, lets its lords'
/// corpses be looted, rests and comes again. `[events] elemental_invasion`
/// off, it stops where it is, and begins again from its first stage when
/// it is back on.
pub fn sweep(realm: &Realm) {
    let t = now();
    let on = realm.config().events.elemental_invasion;
    let changed = {
        let mut s = realm.social.elemental.lock();
        let before = s.driven(t);
        if !on {
            s.started = false;
            let stopped = s.driven(t) != before;
            drop(s);
            if stopped {
                drive(realm);
            }
            return;
        }
        if !s.started || s.rest_until.is_some_and(|r| r <= t) {
            s.begin(t);
        }
        if t >= s.next_hour {
            s.next_hour = t + HOUR;
            for i in 0..4 {
                if (FIRST_STAGE..BOSS_STAGE).contains(&s.stages[i]) {
                    s.stages[i] += 1;
                }
            }
        }
        let all_down = (0..4).all(|i| s.stages[i] >= BOSS_DOWN && !s.looting(i, t));
        if all_down && s.rest_until.is_none() {
            s.rest_until = Some(t + rand::rng().random_range(REST));
        }
        s.driven(t) != before
    };
    if changed {
        drive(realm);
    }
}

/// A creature died: an invader counts against its element, a lord ends it.
pub fn creature_died(map: &mut Map, creature: Guid, entry: u32) {
    let realm = map.realm.clone();
    let lord = ELEMENTS.iter().position(|e| e.lord == entry);
    let invader = ELEMENTS.iter().position(|e| e.invader == entry);
    if lord.is_none() && invader.is_none() {
        return;
    }
    if let Some(i) = invader {
        let mine = map
            .rifts
            .values_mut()
            .any(|r| r.invaders.contains(&creature));
        for r in map.rifts.values_mut() {
            r.invaders.retain(|g| *g != creature);
        }
        if mine {
            realm.social.elemental.lock().slain(i);
            drive(&realm);
        }
    }
    if let Some(i) = lord {
        {
            let mut s = realm.social.elemental.lock();
            s.stages[i] = BOSS_DOWN;
            s.looting_until[i] = Some(now() + LOOTING);
        }
        drive(&realm);
    }
}

/// A rift's invaders.
#[derive(Debug, Clone, Default)]
pub struct Rift {
    pub invaders: Vec<Guid>,
    pub next_upkeep: Option<Instant>,
}

/// Rifts in the map call out the invaders their element's stage wants.
pub fn update(map: &mut Map) {
    let now = map.now;
    if map
        .rifts_checked_at
        .is_some_and(|t| now.duration_since(t) < Duration::from_secs(5))
    {
        return;
    }
    map.rifts_checked_at = Some(now);
    let rifts: Vec<(Guid, usize, Vector3d)> = map
        .gameobjects
        .iter()
        .filter_map(|(_, o)| {
            ELEMENTS
                .iter()
                .position(|e| e.rift == o.entry)
                .map(|i| (o.guid, i, o.position))
        })
        .collect();
    map.rifts
        .retain(|g, _| rifts.iter().any(|(r, _, _)| r == g));
    // An invader gone from the map otherwise than by dying (a script's
    // despawn) is forgotten too, to be called out again (Thistle Tea's
    // `summoned_just_despawn`).
    let gone: Vec<Guid> = map
        .rifts
        .values()
        .flat_map(|r| r.invaders.iter().copied())
        .filter(|g| map.creature(*g).is_none())
        .collect();
    for r in map.rifts.values_mut() {
        r.invaders.retain(|g| !gone.contains(g));
    }
    let stages = map.realm.social.elemental.lock().stages;
    for (rift, i, at) in rifts {
        let due = {
            let r = map.rifts.entry(rift).or_default();
            if r.next_upkeep.is_some_and(|t| t > now) {
                continue;
            }
            r.next_upkeep = Some(now + UPKEEP);
            invaders(stages[i]).saturating_sub(r.invaders.len() as u32)
        };
        for _ in 0..due {
            if let Some(g) = call_invader(map, ELEMENTS[i].invader, at) {
                map.rifts.entry(rift).or_default().invaders.push(g);
            }
        }
    }
    // Invaders out an hour go (only they: Omen, summoned too, has a
    // lifetime of his own).
    let old: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| {
            ELEMENTS.iter().any(|e| e.invader == c.entry)
                && c.summoned_at
                    .is_some_and(|t| now.duration_since(t) > INVADER_LIFETIME)
        })
        .map(|(_, c)| c.guid)
        .collect();
    for g in old {
        for r in map.rifts.values_mut() {
            r.invaders.retain(|x| *x != g);
        }
        map.remove_creature(g);
    }
}

/// An invader out of the rift, roaming 15 to 65 yards from it.
fn call_invader(map: &mut Map, entry: u32, at: Vector3d) -> Option<Guid> {
    let data = map.realm.data.clone();
    let mut rng = rand::rng();
    let angle = rng.random_range(0.0..std::f32::consts::TAU);
    let reach = rng.random_range(15.0f32..65.0);
    let mut pos = Vector3d {
        x: at.x + reach * angle.cos(),
        y: at.y + reach * angle.sin(),
        z: at.z,
    };
    if let Some(nav) = map.nav.as_mut()
        && let Some(z) = nav.ground(pos.x, pos.y, pos.z + 10.0)
    {
        pos.z = z;
    }
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let spawn = crate::game::data::CreatureSpawn {
        guid: counter | 0x80_0000,
        entries: vec![entry],
        map: map.key.map,
        x: pos.x,
        y: pos.y,
        z: pos.z,
        orientation: angle,
        wander_distance: 30.0,
        movement_type: crate::game::map::creature::MOVEMENT_RANDOM,
        ..Default::default()
    };
    let mut c = crate::game::map::creature::Creature::from_spawn(&data, &spawn, &mut rng)?;
    c.spawn_guid = None;
    c.summoned_at = Some(map.now);
    Some(map.add_creature(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap;

    #[test]
    fn stages_climb_by_kills_and_the_lords_end_it() {
        let mut s = State::default();
        s.begin(0);
        assert_eq!(invaders(1), 3);
        assert_eq!(invaders(5), 6);
        for _ in 0..KILLS_PER_STAGE {
            s.slain(0);
        }
        assert_eq!(s.stages[0], 2);
        let on: AHashMap<u32, bool> = s.driven(0).into_iter().collect();
        assert!(on[&INVASION]);
        assert!(on[&68]);
        assert!(!on[&72]);
        s.stages[0] = BOSS_STAGE;
        assert!(s.driven(0).into_iter().collect::<AHashMap<_, _>>()[&72]);
        s.stages = [BOSS_DOWN; 4];
        let on: AHashMap<u32, bool> = s.driven(0).into_iter().collect();
        assert!(!on[&INVASION], "all four lords down");
    }
}
