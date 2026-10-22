//! Visibility's refresh (`map/visibility.rs`): who sees whom worked out
//! again for the players that moved, in a crowd of 100, 880, 2,000 or 3,000
//! on a disc 60 yards across, all in sight of each other, as the movement
//! drill has them (docs/PERFORMANCE.md, "The movement drill").
//!
//! ```sh
//! cargo bench --bench visibility                 # everything
//! cargo bench --bench visibility -- in_step      # a group
//! cargo bench --bench visibility -- --max-time 2 # a quick look
//! ```
//!
//! - `in_step`: the whole crowd moves 9 yards (east, then back west) and
//!   every player's refresh is due in the one update, as when the drill's
//!   crowd runs out and back in step. A sample is that update's visibility.
//! - `random`: one in 23 moves, as at random (a player runs 8 yards in
//!   about 23 updates): a sample is an update's visibility in the drill's
//!   random phase.
//! - `one`: one player moves among the crowd: a refresh's own cost.
//! - `wide`: `in_step` on a disc 300 yards across, as a city's crowd: each
//!   sees about half the others, and many pairs are near the edge of
//!   sight, where the quick path looks.
//!
//! Each as the server does it (`quick`: the quick path, the decisions on
//! the visibility threads, `WVS_BENCH_THREADS` of them, else the machine's
//! cores up to 8), on the map's thread alone (`quick_one_thread`), and as
//! it was before the quick path (`before`: each refresh alone and whole).
//! No budget: every refresh due is done, so a sample is its whole cost.

use std::time::Duration;

use divan::Bencher;
use wow_vanilla_server::game::RealmConfig;
use wow_vanilla_server::game::map::vis_bench::Crowd;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const SIZES: [usize; 4] = [100, 880, 2_000, 3_000];
/// The disc's radius: 60 yards across.
const RADIUS: f32 = 30.0;

fn main() {
    divan::main();
}

#[derive(Clone, Copy)]
enum Way {
    Quick,
    OneThread,
    Before,
}

fn crowd(n: usize, way: Way) -> (tokio::runtime::Runtime, Crowd) {
    crowd_on(n, RADIUS, way)
}

fn crowd_on(n: usize, radius: f32, way: Way) -> (tokio::runtime::Runtime, Crowd) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let threads = std::env::var("WVS_BENCH_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(0);
    let cfg = RealmConfig {
        visibility_budget: Duration::from_secs(3600),
        visibility_threads: match way {
            Way::Quick => threads,
            Way::OneThread | Way::Before => 1,
        },
        ..RealmConfig::default()
    };
    let mut c = {
        let _in = rt.enter();
        Crowd::with_config(cfg, n, radius)
    };
    c.map.visibility_dirty.one_by_one = matches!(way, Way::Before);
    (rt, c)
}

fn in_step(b: Bencher, n: usize, way: Way) {
    let (_rt, mut c) = crowd(n, way);
    b.bench_local(|| {
        c.shift(1, 0, 9.0);
        c.refresh();
    });
}

fn random(b: Bencher, n: usize, way: Way) {
    let (_rt, mut c) = crowd(n, way);
    let mut from = 0;
    b.bench_local(|| {
        c.shift(23, from, 9.0);
        from = (from + 1) % 23;
        c.refresh();
    });
}

fn one(b: Bencher, n: usize, way: Way) {
    let (_rt, mut c) = crowd(n, way);
    let mut from = 0;
    b.bench_local(|| {
        c.shift(n, from, 9.0);
        from = (from + 1) % n;
        c.refresh();
    });
}

mod in_step {
    use super::*;

    #[divan::bench(args = SIZES, sample_count = 20, sample_size = 1)]
    fn quick(b: Bencher, n: usize) {
        in_step(b, n, Way::Quick);
    }

    #[divan::bench(args = SIZES, sample_count = 20, sample_size = 1)]
    fn quick_one_thread(b: Bencher, n: usize) {
        in_step(b, n, Way::OneThread);
    }

    #[divan::bench(args = SIZES, sample_count = 5, sample_size = 1)]
    fn before(b: Bencher, n: usize) {
        in_step(b, n, Way::Before);
    }
}

mod random {
    use super::*;

    #[divan::bench(args = SIZES, sample_count = 50, sample_size = 1)]
    fn quick(b: Bencher, n: usize) {
        random(b, n, Way::Quick);
    }

    #[divan::bench(args = SIZES, sample_count = 50, sample_size = 1)]
    fn quick_one_thread(b: Bencher, n: usize) {
        random(b, n, Way::OneThread);
    }

    #[divan::bench(args = SIZES, sample_count = 20, sample_size = 1)]
    fn before(b: Bencher, n: usize) {
        random(b, n, Way::Before);
    }
}

mod one {
    use super::*;

    #[divan::bench(args = SIZES, sample_count = 200, sample_size = 1)]
    fn quick(b: Bencher, n: usize) {
        one(b, n, Way::Quick);
    }

    #[divan::bench(args = SIZES, sample_count = 100, sample_size = 1)]
    fn before(b: Bencher, n: usize) {
        one(b, n, Way::Before);
    }
}

mod wide {
    use super::*;

    fn wide(b: Bencher, n: usize, way: Way) {
        let (_rt, mut c) = crowd_on(n, 150.0, way);
        b.bench_local(|| {
            c.shift(1, 0, 9.0);
            c.refresh();
        });
    }

    #[divan::bench(args = SIZES, sample_count = 20, sample_size = 1)]
    fn quick(b: Bencher, n: usize) {
        wide(b, n, Way::Quick);
    }

    #[divan::bench(args = SIZES, sample_count = 20, sample_size = 1)]
    fn quick_one_thread(b: Bencher, n: usize) {
        wide(b, n, Way::OneThread);
    }

    #[divan::bench(args = SIZES, sample_count = 5, sample_size = 1)]
    fn before(b: Bencher, n: usize) {
        wide(b, n, Way::Before);
    }
}
