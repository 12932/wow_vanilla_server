//! The navmesh's and the terrain's tiles, read off the map's thread.
//!
//! A tile is read and decoded by a small pool of threads of its own
//! ([`read`]), and handed to its map, which takes it in at its next update
//! ([`update`]): the map never waits on the disk. The tiles around each
//! player are asked for ahead ([`ahead`]), as they arrive and as they
//! move, so that most are in before anything there needs them; those no
//! one has been near for `[world] tile_unload_secs` are let go, and read
//! again if they are needed. Until a tile is in, a
//! query over it answers as a map without one would (`nav.rs`,
//! `terrain.rs`). The tests read tiles at once, as they are asked for
//! (`RealmConfig::tiles_in_background`).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use smallvec::SmallVec;
use tokio::sync::mpsc;

use wow_world_messages::vanilla::Vector3d;

use crate::game::map::Map;
use crate::game::map::nav::tile_of;

/// Threads reading tiles. The reads are mostly the disk's, and a few at
/// once keep it busy.
const READERS: usize = 4;
/// How far around a player, beyond what they see, tiles are asked for: a
/// tile is in before a creature it holds wakes, and walking a few seconds
/// at a run doesn't outpace the reading.
const AHEAD_OF_PLAYERS: f32 = 20.0;
/// How often, in updates, idle tiles are looked for (ten seconds).
const UNLOAD_EVERY: u64 = 200;
/// How long an update spends letting go of idle tiles, once some are found.
const UNLOAD_BUDGET: Duration = Duration::from_millis(2);

type Job = Box<dyn FnOnce() + Send>;

/// Read a tile on the readers' threads (started at the first read).
pub fn read(job: impl FnOnce() + Send + 'static) {
    static READERS_TX: OnceLock<mpsc::UnboundedSender<Job>> = OnceLock::new();
    let tx = READERS_TX.get_or_init(|| {
        // tokio's `mpsc` has one receiver: the readers share it, the one
        // holding the lock waiting for the next job.
        let (tx, rx) = mpsc::unbounded_channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..READERS {
            let rx = rx.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("tiles {i}"))
                .spawn(move || {
                    loop {
                        // The lock is let go before the job runs.
                        let next = rx.lock().blocking_recv();
                        let Some(job) = next else { break };
                        // A tile that panics to read costs itself: the map
                        // goes on without it.
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                    }
                });
            if let Err(e) = spawned {
                tracing::error!("no tile reader: {e}");
            }
        }
        tx
    });
    if tx.send(Box::new(job)).is_err() {
        tracing::error!("the tile readers have gone");
    }
}

/// The ADT tiles (`nav::tile_of`) within `r` yards of `(x, y)`, as eight
/// points on the circle and its centre find them: one to four.
pub fn tiles_around(x: f32, y: f32, r: f32) -> SmallVec<[(i32, i32); 4]> {
    let d = r * std::f32::consts::FRAC_1_SQRT_2;
    let mut tiles = SmallVec::new();
    for (dx, dy) in [
        (0.0, 0.0),
        (r, 0.0),
        (-r, 0.0),
        (0.0, r),
        (0.0, -r),
        (d, d),
        (d, -d),
        (-d, d),
        (-d, -d),
    ] {
        let t = tile_of(x + dx, y + dy);
        if !tiles.contains(&t) {
            tiles.push(t);
        }
    }
    tiles
}

/// Take in the tiles read since the last update, and now and then let go
/// of idle ones.
pub fn update(map: &mut Map) {
    let now = map.now;
    if let Some(nav) = map.nav.as_mut() {
        nav.receive(now);
    }
    if let Some(terrain) = map.terrain.as_mut() {
        terrain.receive(now);
    }
    let idle = map.cfg.tile_unload;
    if idle.is_zero() {
        return;
    }
    if map.stats.updates.is_multiple_of(UNLOAD_EVERY) {
        keep_near_players(map);
        map.tiles_unloading = true;
    }
    // A sweep goes on over as many updates as it takes.
    if map.tiles_unloading {
        let nav = map
            .nav
            .as_mut()
            .map_or((0, false), |n| n.unload_idle_within(idle, UNLOAD_BUDGET));
        let terrain = map
            .terrain
            .as_mut()
            .map_or((0, false), |t| t.unload_idle_within(idle, UNLOAD_BUDGET));
        map.tiles_unloading = nav.1 || terrain.1;
        if nav.0 + terrain.0 > 0 {
            tracing::debug!(
                "{}: let go of {} navmesh and {} terrain tiles",
                map.key,
                nav.0,
                terrain.0
            );
        }
    }
}

/// The tiles around each player are used now: a player standing still asks
/// about nothing, and an awake creature far from every player keeps its
/// own by asking about it.
fn keep_near_players(map: &mut Map) {
    let reach = map.cfg.visibility_yards + AHEAD_OF_PLAYERS;
    let near: Vec<(i32, i32)> = map
        .players
        .values()
        .flat_map(|p| {
            let at = p.position();
            tiles_around(at.x, at.y, reach)
        })
        .collect();
    if let Some(nav) = map.nav.as_mut() {
        for &t in &near {
            nav.keep(t);
        }
    }
    if let Some(terrain) = map.terrain.as_mut() {
        for &t in &near {
            terrain.keep(t);
        }
    }
}

/// Ask ahead for the tiles around a player at `at`: as they arrive, and
/// each time they have moved far enough for what they see to be worked
/// out again (`visibility::refresh`). The creatures a player wakes are
/// within what they see, and wander, chase and go home about as far, so
/// this is ahead of the awake creatures too; one further off asks for its
/// own tile as it goes.
pub fn ahead(map: &mut Map, at: Vector3d) {
    if map.nav.is_none() && map.terrain.is_none() {
        return;
    }
    let reach = map.cfg.visibility_yards + AHEAD_OF_PLAYERS;
    let tiles = tiles_around(at.x, at.y, reach);
    if let Some(nav) = map.nav.as_mut() {
        for &t in &tiles {
            nav.want(t);
        }
    }
    if let Some(terrain) = map.terrain.as_mut() {
        for &t in &tiles {
            terrain.want(t);
        }
    }
}
