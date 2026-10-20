//! Channel chat's fan-out (`social/channels.rs`, `channels::say`, run on
//! the speaker's map task): a line said in a channel of 100, 1,000 or 5,000
//! members, from 1 map or 8 at once, through the real realm, registry,
//! channels and `Outbound`s, each member's queue drained by a writer task
//! of its own.
//!
//! ```sh
//! cargo bench --bench chat                   # everything: 18 cases, about two minutes
//! cargo bench --bench chat -- delivered      # a group: delivered or said
//! cargo bench --bench chat -- --max-time 0.5 # a quick look
//! ```
//!
//! A case is rounds: a round says lines until 40,000 deliveries are due,
//! the same whatever the members and the maps. In `delivered` a round ends
//! when the writers have taken every delivery: its `item/s` is the
//! deliveries in a second, end to end. In `said` it ends when the maps are
//! done saying (the writers working meanwhile, as they do), and the
//! writers' leftovers are drained before the next, untimed: its `item/s` is
//! what a map hands out in a second, the time its update isn't running.
//! How to read divan's output is in docs/PERFORMANCE.md ("The concurrency
//! micro-benchmarks").

use std::cell::Cell;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::thread;

use divan::Bencher;
use divan::counter::ItemsCount;
use wow_vanilla_server::game::conn::{Frames, Outbound};
use wow_vanilla_server::game::data::GameData;
use wow_vanilla_server::game::map::MapKey;
use wow_vanilla_server::game::registry::{NameEntry, Online};
use wow_vanilla_server::game::social::channels;
use wow_vanilla_server::game::store::Store;
use wow_vanilla_server::game::{Realm, RealmConfig, guid};
use wow_world_messages::vanilla::{Language, PlayerChatTag};

// As the server's binaries have it.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Seconds each case runs.
const SECS: f64 = 4.0;
/// The deliveries in a round: a line to every member of its channel, the
/// speaker too, as many lines as make this many.
const DELIVERIES: usize = 40_000;
/// `run_writer`'s batch.
const BATCH_LIMIT: usize = 128;
/// A line, as long as a trade channel's.
const LINE: &str = "WTS [Thunderfury, Blessed Blade of the Windseeker] PST, 2k or best offer";

fn main() {
    divan::main();
}

/// A tokio runtime as the server starts one (`server.rs`), a worker a core,
/// but for the core the benchmark's thread spins on; `WVS_BENCH_WORKERS`
/// sets another count.
fn runtime() -> tokio::runtime::Runtime {
    let cores = thread::available_parallelism().map_or(8, |n| n.get());
    let workers = std::env::var("WVS_BENCH_WORKERS")
        .ok()
        .and_then(|w| w.parse().ok())
        .unwrap_or(cores.saturating_sub(1).max(1));
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .expect("a tokio runtime")
}

/// Spin until `done`. Never yield: on a busy machine a yield can give the
/// core away for milliseconds, and the round's time would be the wait for
/// it back.
fn wait_until(done: impl Fn() -> bool) {
    while !done() {
        std::hint::spin_loop();
    }
}

/// A counter on a cache line of its own.
#[derive(Default)]
#[repr(align(128))]
struct Shard(AtomicU64);

/// What the writers have taken, in all the rounds: sharded, so that
/// thousands of writers counting don't queue on one cache line, which would
/// be the harness's cost, not wvs's.
#[derive(Default)]
struct Tally([Shard; 16]);

impl Tally {
    fn add(&self, writer: usize, n: usize) {
        self.0[writer % self.0.len()]
            .0
            .fetch_add(n as u64, Ordering::Release);
    }

    fn total(&self) -> u64 {
        self.0.iter().map(|s| s.0.load(Ordering::Acquire)).sum()
    }
}

/// `run_writer` without the cipher and the socket: wait for a frame, take
/// what else is queued up to [`BATCH_LIMIT`], copy them out.
async fn writer(mut frames: Frames, tally: Arc<Tally>, id: usize) {
    let mut scratch: Vec<u8> = Vec::with_capacity(16 * 1024);
    while let Some(first) = frames.recv().await {
        scratch.clear();
        scratch.extend_from_slice(&first);
        let mut n = 1;
        while n < BATCH_LIMIT {
            let Some(f) = frames.try_recv() else {
                break;
            };
            scratch.extend_from_slice(&f);
            n += 1;
        }
        black_box(&scratch);
        tally.add(id, n);
    }
}

/// A realm with `channels` channels of `members` each (built-in ones, as
/// General and Trade are: no notices at a join), every member online with
/// a writer, and `speakers` of each channel's members to speak in it.
struct World {
    realm: Arc<Realm>,
    tally: Arc<Tally>,
    /// The speakers, a map each, and the channel each speaks in.
    speakers: Vec<(Arc<Online>, Arc<str>)>,
    /// The speakers' maps done with a round, in all the rounds.
    said: Arc<AtomicU64>,
    /// The rounds started.
    rounds: Cell<u64>,
    /// The writers' tally before the first round.
    joined: u64,
    /// The lines each speaker says in a round.
    lines: usize,
    rt: tokio::runtime::Runtime,
}

impl World {
    fn new(channels: usize, members: usize, speakers: usize) -> Self {
        let rt = runtime();
        let _in = rt.enter();
        let (store, startup) = Store::open(":memory:").expect("an in-memory store");
        let realm = Realm::new(
            RealmConfig::default(),
            store,
            startup,
            GameData::empty(),
            None,
        );
        let map = realm.maps.get(&realm, MapKey::world(0));
        let tally = Arc::new(Tally::default());
        let mut all_speakers = Vec::new();
        for c in 0..channels {
            let name: Arc<str> = Arc::from(format!("General - Bench {c}"));
            for i in 0..members {
                let id = c * members + i + 1;
                let entry = realm.registry.add_name(NameEntry {
                    guid: guid::player(id as u32),
                    account: Arc::from(format!("A{id}")),
                    name: Arc::from(format!("P{id}")),
                    race: 1,
                    class: 1,
                    gender: 0,
                    level: AtomicU8::new(60),
                });
                let (outbound, frames) = Outbound::detached();
                let (session, _) = tokio::sync::mpsc::unbounded_channel();
                let online = Arc::new(Online::new(&entry, 0, outbound, session, map.clone()));
                realm
                    .registry
                    .go_online(online.clone())
                    .expect("a new player");
                rt.spawn(writer(frames, tally.clone(), id));
                channels::join(&realm, &online, &name, "");
                if i < speakers {
                    all_speakers.push((online, name.clone()));
                }
            }
        }
        // Each member's YOU_JOINED.
        let joined = (channels * members) as u64;
        wait_until(|| tally.total() >= joined);
        let lines = DELIVERIES / (all_speakers.len() * members);
        assert_eq!(lines * all_speakers.len() * members, DELIVERIES);
        drop(_in);
        Self {
            realm,
            joined: tally.total(),
            tally,
            speakers: all_speakers,
            said: Arc::default(),
            rounds: Cell::new(0),
            lines,
            rt,
        }
    }

    /// Each speaker's map (a task, as a map is) says its lines, all at once.
    fn say(&self) {
        self.rounds.set(self.rounds.get() + 1);
        for (online, name) in &self.speakers {
            let (realm, online, name) = (self.realm.clone(), online.clone(), name.clone());
            let (lines, said) = (self.lines, self.said.clone());
            self.rt.spawn(async move {
                for _ in 0..lines {
                    channels::say(
                        &realm,
                        &online,
                        &name,
                        LINE,
                        Language::Universal,
                        PlayerChatTag::None,
                    );
                }
                said.fetch_add(1, Ordering::Release);
            });
        }
    }

    /// Until the speakers' maps are done with every round.
    fn wait_said(&self) {
        let done = self.rounds.get() * self.speakers.len() as u64;
        wait_until(|| self.said.load(Ordering::Acquire) >= done);
    }

    /// Until the writers have taken every delivery of every round.
    fn wait_delivered(&self) {
        let total = self.joined + self.rounds.get() * DELIVERIES as u64;
        wait_until(|| self.tally.total() >= total);
    }
}

/// A round to the end: said, and every delivery taken by its writer.
fn to_the_end(bencher: Bencher, world: &World) {
    bencher
        .counter(ItemsCount::new(DELIVERIES))
        .bench_local(|| {
            world.say();
            world.wait_delivered();
        });
}

/// A round as the maps see it: until they are done saying; what the
/// writers have left is drained before the next, untimed.
fn as_said(bencher: Bencher, world: &World) {
    bencher
        .counter(ItemsCount::new(DELIVERIES))
        .with_inputs(|| world.wait_delivered())
        .bench_local_values(|()| {
            world.say();
            world.wait_said();
        });
}

/// End to end: a round ends when every delivery is taken.
#[divan::bench_group(min_time = SECS, max_time = SECS)]
mod delivered {
    use super::*;

    /// One channel of `members`, said in by `MAPS` maps at once.
    #[divan::bench(consts = [1, 8], args = [100, 1000, 5000])]
    fn one_channel<const MAPS: usize>(bencher: Bencher, members: usize) {
        to_the_end(bencher, &World::new(1, members, MAPS));
    }

    /// Eight channels of `members` each, eight maps each saying lines in
    /// its own: what two channels' chat costs each other.
    #[divan::bench(args = [100, 1000, 5000])]
    fn own_channels(bencher: Bencher, members: usize) {
        to_the_end(bencher, &World::new(8, members, 1));
    }
}

/// The maps' side: a round ends when the maps are done saying.
#[divan::bench_group(min_time = SECS, max_time = SECS, sample_size = 1)]
mod said {
    use super::*;

    /// One channel of `members`, said in by `MAPS` maps at once.
    #[divan::bench(consts = [1, 8], args = [100, 1000, 5000])]
    fn one_channel<const MAPS: usize>(bencher: Bencher, members: usize) {
        as_said(bencher, &World::new(1, members, MAPS));
    }

    /// Eight channels of `members` each, eight maps each in its own.
    #[divan::bench(args = [100, 1000, 5000])]
    fn own_channels(bencher: Bencher, members: usize) {
        as_said(bencher, &World::new(8, members, 1));
    }
}
