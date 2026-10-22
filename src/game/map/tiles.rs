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
use crate::game::map::nav::{ADT_SIZE, tile_of};

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

/// [`read`] `job` for the tile `key`, and send `tx` the key and what it
/// read. A read that panics sends `missing`: the map takes the tile as not
/// to be had, where it would wait on it, loading, until the map restarted.
pub fn read_into<K, V>(
    tx: mpsc::UnboundedSender<(K, V)>,
    key: K,
    missing: V,
    job: impl FnOnce() -> V + Send + 'static,
) where
    K: Send + 'static,
    V: Send + 'static,
{
    read(move || {
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
        let _ = tx.send((key, read.unwrap_or(missing)));
    });
}

/// The ADT tiles (`nav::tile_of`) within `r` yards of `(x, y)`: every one
/// whose square the disc touches (up to nine, for a disc wider than a
/// tile), the one it is centred in first.
pub fn tiles_around(x: f32, y: f32, r: f32) -> SmallVec<[(i32, i32); 4]> {
    let mut tiles = SmallVec::new();
    let centre = tile_of(x, y);
    tiles.push(centre);
    // `tile_of` runs against the axes: the box's high corner has the low
    // tiles.
    let (tx0, ty0) = tile_of(x + r, y + r);
    let (tx1, ty1) = tile_of(x - r, y - r);
    for tx in tx0..=tx1 {
        for ty in ty0..=ty1 {
            if (tx, ty) == centre {
                continue;
            }
            // Tile `(tx, ty)` covers y in `(31 - tx) * S ..= (32 - tx) * S`,
            // and x likewise by `ty`.
            let ylo = (31 - tx) as f32 * ADT_SIZE;
            let xlo = (31 - ty) as f32 * ADT_SIZE;
            let nx = x.clamp(xlo, xlo + ADT_SIZE);
            let ny = y.clamp(ylo, ylo + ADT_SIZE);
            if (nx - x).powi(2) + (ny - y).powi(2) <= r * r {
                tiles.push((tx, ty));
            }
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
    let near: Vec<(i32, i32)> = map
        .players
        .values()
        .flat_map(|p| {
            let at = p.position();
            let reach = crate::game::map::visibility::sight(map, p) + AHEAD_OF_PLAYERS;
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
/// own tile as it goes. `sees` is how far the player sees
/// (`visibility::sight`: an instance's, a battleground's, a flight's).
pub fn ahead(map: &mut Map, at: Vector3d, sees: f32) {
    if map.nav.is_none() && map.terrain.is_none() {
        return;
    }
    let reach = sees + AHEAD_OF_PLAYERS;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug hunt's map core: nine points on and in the circle missed a
    /// diagonal tile whose corner was within reach. A centre 100 yards
    /// west and 10 south of a tile corner, with 101 yards' reach, has the
    /// corner's diagonal tile; and a disc wider than a tile has all nine.
    #[test]
    fn the_tiles_around_are_every_one_the_disc_touches() {
        let corner = (-ADT_SIZE, -ADT_SIZE);
        let (x, y) = (corner.0 - 10.0, corner.1 - 100.0);
        let tiles = tiles_around(x, y, 101.0);
        let diagonal = tile_of(corner.0 + 1.0, corner.1 + 1.0);
        assert!(tiles.contains(&diagonal), "{tiles:?}");
        assert_eq!(tiles.len(), 4, "{tiles:?}");
        assert_eq!(tiles[0], tile_of(x, y));
        // Not past its reach.
        assert_eq!(tiles_around(x, y, 99.0).len(), 2);
        let mid = -ADT_SIZE / 2.0;
        assert_eq!(tiles_around(mid, mid, 200.0).len(), 1);
        assert_eq!(tiles_around(mid, mid, 300.0).len(), 5);
        assert_eq!(tiles_around(mid, mid, 420.0).len(), 9);
    }

    /// rev5-data 8: a tile whose read panics is answered as missing, so its
    /// map doesn't wait on it, loading, for good.
    #[test]
    fn a_read_that_panics_answers_missing() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        read_into(tx.clone(), (1, 2), None, || -> Option<u32> {
            panic!("a tile that doesn't decode")
        });
        read_into(tx, (3, 4), None, || Some(7));
        let mut got = [
            rx.blocking_recv().expect("an answer"),
            rx.blocking_recv().expect("an answer"),
        ];
        got.sort();
        assert_eq!(got, [((1, 2), None), ((3, 4), Some(7))]);
    }
}
