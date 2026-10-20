//! A city's fireworks show, as vmangos's `go_cheer_speaker` (Thistle Tea's
//! `CheerSpeaker` and `FireworksLaunch`) runs it for as long as the
//! fireworks event (6) keeps the speakers out: a cheer as the show begins
//! and as it ends, and between them a random firework sent up over the
//! speaker's city every one to two seconds, from one of its launch sites.
//! A speaker belongs to the city whose sites lie nearest it on its map,
//! within 250 yards; one near none sends nothing up.
//!
//! The opening cheer waits for the first firework, a second or two on:
//! a sound from an object reaches only clients that have it, and none has
//! the speaker as it comes.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{SMSG_PLAY_OBJECT_SOUND, Vector3d};

use crate::game::cheer_sites::{CHEERS, CITIES, FIREWORKS};
use crate::game::map::Map;

pub const SPEAKER: u32 = 180_749;
/// A speaker's city is the nearest whose sites are within this.
const CITY_REACH: f32 = 250.0;
pub const INTERVAL_MS: std::ops::RangeInclusive<u64> = 1_000..=2_000;
/// How long a firework's object stays to burst.
pub const FIREWORK_LIFETIME_MS: u32 = 500;

/// A speaker's show.
#[derive(Debug, Clone)]
pub struct Speaker {
    pub next_launch: Instant,
    /// Its opening cheer, still to come.
    pub cheer_due: bool,
}

/// The launch sites of the city a speaker at `at` on `map_id` belongs to.
pub fn sites(map_id: u32, at: Vector3d) -> &'static [[f32; 4]] {
    CITIES
        .iter()
        .filter(|(m, _)| *m == map_id)
        .map(|(_, sites)| {
            let nearest = sites
                .iter()
                .map(|s| ((s[0] - at.x).powi(2) + (s[1] - at.y).powi(2)).sqrt())
                .fold(f32::INFINITY, f32::min);
            (nearest, *sites)
        })
        .filter(|(d, _)| *d <= CITY_REACH)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(&[], |(_, sites)| sites)
}

fn next(now: Instant) -> Instant {
    now + Duration::from_millis(rand::rng().random_range(INTERVAL_MS))
}

/// An object came into the map: a speaker's show begins.
pub fn added(map: &mut Map, g: Guid, entry: u32) {
    if entry == SPEAKER {
        let now = map.now;
        map.speakers.insert(
            g,
            Speaker {
                next_launch: next(now),
                cheer_due: true,
            },
        );
    }
}

/// An object leaves the map (its event over): a speaker cheers its show's
/// end to those who have it.
pub fn removed(map: &mut Map, g: Guid) {
    if map.speakers.remove(&g).is_some() {
        cheer(map, g);
    }
}

fn cheer(map: &Map, g: Guid) {
    let sound = CHEERS[rand::rng().random_range(0..CHEERS.len())];
    if let Some(frame) = crate::game::conn::encode(&SMSG_PLAY_OBJECT_SOUND {
        sound_id: sound,
        guid: g,
    }) {
        map.send_to_observers(g, &frame, false);
    }
}

/// Each speaker whose time has come sends a firework up over its city.
pub fn update(map: &mut Map) {
    if map.speakers.is_empty() {
        return;
    }
    let now = map.now;
    let due: Vec<(Guid, bool)> = map
        .speakers
        .iter()
        .filter(|(_, s)| s.next_launch <= now)
        .map(|(g, s)| (*g, s.cheer_due))
        .collect();
    for (g, cheer_due) in due {
        if let Some(s) = map.speakers.get_mut(&g) {
            s.next_launch = next(now);
            s.cheer_due = false;
        }
        if cheer_due {
            cheer(map, g);
        }
        launch(map, g);
    }
}

/// A random firework at a random site of the speaker's city.
pub fn launch(map: &mut Map, g: Guid) -> Option<Guid> {
    let at = map.gameobject(g)?.position;
    let sites = sites(map.key.map, at);
    if sites.is_empty() {
        return None;
    }
    let (site, firework) = {
        let mut rng = rand::rng();
        (
            sites[rng.random_range(0..sites.len())],
            FIREWORKS[rng.random_range(0..FIREWORKS.len())],
        )
    };
    crate::game::map::objects::summon(
        map,
        None,
        firework,
        Vector3d {
            x: site[0],
            y: site[1],
            z: site[2],
        },
        site[3],
        FIREWORK_LIFETIME_MS,
        0,
        0,
        None,
    )
}
