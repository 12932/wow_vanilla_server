//! Weather, as vmangos and Thistle Tea run it: every ten minutes each zone
//! with `game_weather` chances may change (seven times in ten). Fair
//! weather, or light weather that gets better, turns to rain, snow or a
//! storm by the season's chances, or stays fair; other weather gets better
//! or worse by a third, or changes all at once. Those in the zone are told.
//! `[world] weather` and `weather_change_secs` turn it off and space it.

use std::time::Duration;

use ahash::AHashMap;
use rand::RngExt;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::registry::Online;

/// How often a zone's weather may change, by default (`[world]
/// weather_change_secs`).
pub const CHANGE_EVERY: Duration = Duration::from_secs(600);

/// A third of the way: a step better or worse.
const STEP: f32 = 0.333_333_34;
/// Two thirds: heavy weather.
const HEAVY: f32 = 0.666_666_7;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weather {
    pub kind: u8,
    pub grade: f32,
    /// Unix seconds.
    pub next: i64,
}

impl Default for Weather {
    fn default() -> Self {
        Self {
            kind: 0,
            grade: 0.0,
            next: 0,
        }
    }
}

/// The season by the day of the year (0 for 1 January), as mangos counts
/// it: spring from the 79th day (19 March, or the 18th in a leap year),
/// then 91 days a season. Spring, summer, fall, winter.
pub fn season(day0: u32) -> usize {
    ((day0 + 365 - 78) / 91 % 4) as usize
}

/// A change's rolls, as mangos's `ReGenerate` makes them: whether and how
/// it changes (0-99), the new kind (1-100), a radical change's (0-99) and a
/// new heavy weather's (0-99), and the grade within its third (0-1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rolls {
    pub change: u32,
    pub kind: u32,
    pub radical: u32,
    pub intensity: u32,
    pub grade: f32,
}

/// The next weather from the season's chances (in percent) and the rolls
/// (Thistle Tea's `Weather.advance`, mangos's `ReGenerate`): three times in
/// ten none; light weather three in ten gives way to the season's, else
/// weather gets better by a third; three in ten it gets worse; and one in
/// ten light goes heavy, heavy half the time light, and anything else
/// gives way to the season's.
pub fn advance(kind: u8, grade: f32, chances: [u32; 3], r: Rolls) -> (u8, f32) {
    if r.change < 30 {
        return (kind, grade);
    }
    let fine = kind == 0;
    let next = if r.change < 60 && grade < STEP {
        choose(chances, r)
    } else if r.change < 60 && !fine {
        (kind, grade - STEP)
    } else if r.change < 90 && !fine {
        (kind, grade + STEP)
    } else if !fine && grade < STEP {
        (kind, 0.9999)
    } else if !fine && grade > HEAVY && r.radical < 50 {
        (kind, grade - HEAVY)
    } else {
        choose(chances, r)
    };
    normalize(next)
}

/// New weather by the season's chances: light, or on a radical change
/// medium or heavy half the time each.
fn choose(chances: [u32; 3], r: Rolls) -> (u8, f32) {
    let (rain, snow, storm) = (chances[0], chances[1], chances[2]);
    let kind = if r.kind <= rain {
        1
    } else if r.kind <= rain + snow {
        2
    } else if r.kind <= rain + snow + storm {
        3
    } else {
        0
    };
    let offset = if r.change < 90 {
        0.0
    } else if r.intensity < 50 {
        0.3334
    } else {
        0.6667
    };
    (kind, r.grade * 0.3333 + offset)
}

/// A grade the client takes: none for fair weather, else short of 1 and
/// above 0 (mangos's `NormalizeGrade`).
fn normalize((kind, grade): (u8, f32)) -> (u8, f32) {
    if kind == 0 {
        (0, 0.0)
    } else if grade >= 1.0 {
        (kind, 0.9999)
    } else if grade < 0.0 {
        (kind, 0.0001)
    } else {
        (kind, grade)
    }
}

/// The sound of the weather, as vmangos's `GetSound` and Thistle Tea have
/// it: none under 0.3, light under 0.6, medium under 0.9, else heavy.
pub fn sound(kind: u8, grade: f32) -> u32 {
    let base = match kind {
        1 => 8533,
        2 => 8536,
        3 => 8556,
        _ => return 0,
    };
    if grade < 0.3 {
        0
    } else if grade < 0.6 {
        base
    } else if grade < 0.9 {
        base + 1
    } else {
        base + 2
    }
}

fn packet(w: Weather) -> SMSG_WEATHER {
    SMSG_WEATHER {
        weather_type: WeatherType::try_from(u32::from(w.kind)).unwrap_or(WeatherType::Fine),
        grade: w.grade,
        sound_id: sound(w.kind, w.grade),
        change: WeatherChangeType::Smooth,
    }
}

/// Set a zone's weather (a GM's `.weather`): `kind` 0 fine, 1 rain, 2 snow,
/// 3 storm; held a change period, or for good. Those in the zone see it.
pub fn set(realm: &Realm, zone: u32, kind: u8, grade: f32, permanent: bool) {
    let now = crate::game::social::mail::now();
    let every = realm.config().weather_change.as_secs() as i64;
    let (kind, grade) = normalize((kind, grade.clamp(0.0, 1.0)));
    let w = Weather {
        kind,
        grade,
        next: if permanent { i64::MAX } else { now + every },
    };
    realm.social.weather.lock().insert(zone, w);
    for o in realm.registry.all_online() {
        if o.zone() == zone {
            o.outbound.send(&packet(w));
        }
    }
}

/// A zone's weather back to the season's rolls.
pub fn release(realm: &Realm, zone: u32) {
    if let Some(w) = realm.social.weather.lock().get_mut(&zone) {
        w.next = 0;
    }
}

/// The zone's weather to one player, on entering it.
pub fn tell(realm: &Realm, o: &Online, zone: u32) {
    let w = realm
        .social
        .weather
        .lock()
        .get(&zone)
        .copied()
        .unwrap_or_default();
    o.outbound.send(&packet(w));
}

/// Zones due a change roll for it; those in them hear of changes. With the
/// weather off, what was rolled clears, and what a GM set for good stays.
pub fn sweep(realm: &Realm) {
    use chrono::Datelike;
    let now = crate::game::social::mail::now();
    let (on, every) = {
        let cfg = realm.config();
        (cfg.weather, cfg.weather_change.as_secs() as i64)
    };
    let s = season(chrono::Local::now().ordinal0());
    let mut rng = rand::rng();
    let mut changed: Vec<(u32, Weather)> = Vec::new();
    {
        let mut all = realm.social.weather.lock();
        for (zone, chances) in &realm.data.weather {
            let w: &mut Weather = all.entry(*zone).or_default();
            if !on {
                if w.next != i64::MAX && (w.kind, w.grade) != (0, 0.0) {
                    *w = Weather::default();
                    changed.push((*zone, *w));
                }
                continue;
            }
            if w.next > now {
                continue;
            }
            let rolls = Rolls {
                change: rng.random_range(0..100),
                kind: rng.random_range(1..=100),
                radical: rng.random_range(0..100),
                intensity: rng.random_range(0..100),
                grade: rng.random_range(0.0..1.0),
            };
            let (kind, grade) = advance(w.kind, w.grade, chances[s], rolls);
            w.next = now + every;
            if (kind, grade) != (w.kind, w.grade) {
                w.kind = kind;
                w.grade = grade;
                changed.push((*zone, *w));
            }
        }
    }
    if changed.is_empty() {
        return;
    }
    let by_zone: AHashMap<u32, Weather> = changed.into_iter().collect();
    for o in realm.registry.all_online() {
        if let Some(w) = by_zone.get(&o.zone()) {
            o.outbound.send(&packet(*w));
        }
    }
}
