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

use divan::Bencher;
use wow_vanilla_server::game::map::vis_bench::Crowd;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const SIZES: [usize; 4] = [100, 880, 2_000, 3_000];
/// The disc's radius: 60 yards across.
const RADIUS: f32 = 30.0;

fn main() {
    divan::main();
}

fn crowd(n: usize) -> (tokio::runtime::Runtime, Crowd) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let c = {
        let _in = rt.enter();
        Crowd::new(n, RADIUS)
    };
    (rt, c)
}

#[divan::bench(args = SIZES, sample_count = 10, sample_size = 1)]
fn in_step(b: Bencher, n: usize) {
    let (_rt, mut c) = crowd(n);
    b.bench_local(|| {
        c.shift(1, 0, 9.0);
        c.refresh();
    });
}

#[divan::bench(args = SIZES, sample_count = 50, sample_size = 1)]
fn random(b: Bencher, n: usize) {
    let (_rt, mut c) = crowd(n);
    let mut from = 0;
    b.bench_local(|| {
        c.shift(23, from, 9.0);
        from = (from + 1) % 23;
        c.refresh();
    });
}

#[divan::bench(args = SIZES, sample_count = 200, sample_size = 1)]
fn one(b: Bencher, n: usize) {
    let (_rt, mut c) = crowd(n);
    let mut from = 0;
    b.bench_local(|| {
        c.shift(n, from, 9.0);
        from = (from + 1) % n;
        c.refresh();
    });
}
