//! The update's values (`Map::flush_values` and `Map::flush_batches`): a
//! crowd of 100 to 3,000 players on a disc 80 yards across, all in sight
//! of each other, as a battle's in one spot, some or all of them changed
//! since the last update (health, as blows and ticks leave it).
//!
//! ```sh
//! cargo bench --bench values                  # everything
//! cargo bench --bench values -- shared        # a way
//! cargo bench --bench values -- --max-time 2  # a quick look
//! ```
//!
//! - `shared`: as the server does it (`map::values`): each changed
//!   player's block kept once, put in packets shared by those who get them
//!   alike, broadcast through the map's feed.
//! - `one_by_one`: as before: each block copied into every observer's
//!   batch, each batch cut, hashed and framed.
//!
//! `all` changes every player each update, `third` a third of them. A
//! sample is one update's values: the flush and the batches' frames, the
//! clients' queues emptied between samples, untimed.

use std::cell::RefCell;

use divan::Bencher;
use wow_vanilla_server::game::map::vis_bench::Crowd;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const SIZES: [usize; 4] = [100, 800, 1_600, 3_000];
/// The disc's radius: 80 yards across, the battle's.
const RADIUS: f32 = 40.0;

fn main() {
    divan::main();
}

fn crowd(n: usize, one_by_one: bool) -> (tokio::runtime::Runtime, RefCell<Crowd>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let mut c = {
        let _in = rt.enter();
        Crowd::new(n, RADIUS)
    };
    c.values_one_by_one(one_by_one);
    (rt, RefCell::new(c))
}

fn run(bencher: Bencher, n: usize, one_by_one: bool, every: usize) {
    let (rt, c) = crowd(n, one_by_one);
    let _in = rt.enter();
    let mut step = 0u32;
    bencher
        .with_inputs(|| {
            let mut c = c.borrow_mut();
            c.drain();
            step += 1;
            c.hurt(every, step as usize % every, step);
        })
        .bench_local_values(|()| c.borrow_mut().flush_values());
}

#[divan::bench(args = SIZES, sample_count = 30)]
fn shared_all(bencher: Bencher, n: usize) {
    run(bencher, n, false, 1);
}

#[divan::bench(args = SIZES, sample_count = 30)]
fn one_by_one_all(bencher: Bencher, n: usize) {
    run(bencher, n, true, 1);
}

#[divan::bench(args = SIZES, sample_count = 30)]
fn shared_third(bencher: Bencher, n: usize) {
    run(bencher, n, false, 3);
}

#[divan::bench(args = SIZES, sample_count = 30)]
fn one_by_one_third(bencher: Bencher, n: usize) {
    run(bencher, n, true, 3);
}
