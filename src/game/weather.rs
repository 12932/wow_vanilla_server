//! Weather, as mangos runs it: every ten minutes each zone with
//! `game_weather` chances may change (seven times in ten), to rain, snow
//! or a storm by the season's chances, or fair. Those in the zone are told.

use std::time::Duration;

use ahash::AHashMap;
use rand::RngExt;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::registry::Online;

pub const CHANGE_EVERY: Duration = Duration::from_secs(600);

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

/// The season by month: spring, summer, fall, winter.
pub fn season(month0: u32) -> usize {
    match month0 {
        2..=4 => 0,
        5..=7 => 1,
        8..=10 => 2,
        _ => 3,
    }
}

/// The next weather from the season's chances (in percent) and two rolls.
pub fn roll(chances: [u32; 3], keep: u32, pick: u32, grade: f32, old: Weather) -> (u8, f32) {
    if keep < 30 {
        return (old.kind, old.grade);
    }
    let (rain, snow, storm) = (chances[0], chances[1], chances[2]);
    let kind = if pick < rain {
        1
    } else if pick < rain + snow {
        2
    } else if pick < rain + snow + storm {
        3
    } else {
        0
    };
    (kind, if kind == 0 { 0.0 } else { grade })
}

fn packet(w: Weather) -> SMSG_WEATHER {
    let tier = (w.grade * 3.0).min(2.0) as u32;
    let sound = match w.kind {
        1 => 8533 + tier,
        2 => 8536 + tier,
        3 => 8556 + tier,
        _ => 0,
    };
    SMSG_WEATHER {
        weather_type: WeatherType::try_from(u32::from(w.kind)).unwrap_or(WeatherType::Fine),
        grade: w.grade,
        sound_id: sound,
        change: WeatherChangeType::Smooth,
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

/// Zones due a change roll for it; those in them hear of changes.
pub fn sweep(realm: &Realm) {
    use chrono::Datelike;
    let now = crate::game::social::mail::now();
    let s = season(chrono::Local::now().month0());
    let mut rng = rand::rng();
    let mut changed: Vec<(u32, Weather)> = Vec::new();
    {
        let mut all = realm.social.weather.lock();
        for (zone, chances) in &realm.data.weather {
            let w: &mut Weather = all.entry(*zone).or_default();
            if w.next > now {
                continue;
            }
            let (kind, grade) = roll(
                chances[s],
                rng.random_range(0..100),
                rng.random_range(0..100),
                rng.random_range(0.0..1.0),
                *w,
            );
            w.next = now + CHANGE_EVERY.as_secs() as i64;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_keeps_three_times_in_ten_and_follows_the_chances() {
        let old = Weather {
            kind: 1,
            grade: 0.5,
            next: 0,
        };
        assert_eq!(roll([25, 0, 0], 10, 0, 0.9, old), (1, 0.5));
        assert_eq!(roll([25, 0, 0], 50, 10, 0.9, old), (1, 0.9));
        assert_eq!(roll([25, 10, 5], 50, 30, 0.9, old), (2, 0.9));
        assert_eq!(roll([25, 10, 5], 50, 38, 0.9, old), (3, 0.9));
        assert_eq!(roll([25, 10, 5], 50, 90, 0.9, old), (0, 0.0));
        assert_eq!(season(0), 3);
        assert_eq!(season(6), 1);
    }
}
