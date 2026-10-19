//! Micro-benchmarks for wvs's concurrency choices: which channel, which
//! lock, and how the live settings are read. Each benchmark stands for a
//! path in wvs, named in its doc comment, and runs that path's shape (who
//! sends, from where, who receives and how) with each candidate in turn.
//!
//! ```sh
//! cargo bench --bench sync                     # everything: about five minutes
//! cargo bench --bench sync -- channels         # a group: channels, locks or live_config
//! cargo bench --bench sync -- channels::store  # one benchmark
//! cargo bench --bench sync -- --max-time 0.2   # a quick look: each case 0.2 s
//! ```
//!
//! Each case (a candidate at a thread or producer count) runs for
//! [`SECS`]. How to read divan's output is in docs/PERFORMANCE.md ("The
//! concurrency micro-benchmarks").
//!
//! The channels' cases are rounds: a round sends a fixed number of items
//! and ends when the consumer has taken them all, so a row's time is a
//! round's, and its `item/s` the items through a second. The locks' and the
//! live settings' cases run on divan's threads (`t=N`): a row's time is one
//! operation on one thread while the others run theirs.

use std::future::Future;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};

use divan::Bencher;
use divan::counter::ItemsCount;

// As the server's binaries have it: the channels' and the frames'
// allocations are mimalloc's.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Seconds each case runs: 76 cases, about five minutes in all.
const SECS: f64 = 4.0;
/// The operations each thread times in a sample, on divan's threads. Fixed:
/// divan's own choice goes by the slowest thread's sample, and under
/// contention stops at one operation a sample, which times the sample's
/// start (every thread at once, from a barrier) and the clock, not a lock
/// in use.
const OPS_A_SAMPLE: u32 = 4096;

fn main() {
    divan::main();
}

// The candidates. A benchmark is generic over them (divan's `types`), and
// each module says what each is for its path.

/// tokio's channels: `mpsc`, and `oneshot` for a reply.
pub struct Tokio;
/// tokio's `mpsc` drained with `recv_many`, its own batched receive.
pub struct TokioRecvMany;
/// tokio's `mpsc` in a loop of its own, without `select!`.
pub struct TokioNoSelect;
/// kanal's channels.
pub struct Kanal;
/// kanal's async channel in a loop of its own: its receive isn't safe in
/// `select!`.
pub struct KanalNoSelect;
/// flume's channels.
pub struct Flume;
/// crossbeam-channel's.
pub struct Crossbeam;
/// The standard library's (`std::sync::mpsc`, `std::sync`'s locks).
pub struct Std;
/// std's locks taken whatever a panic left: `unwrap_or_else(PoisonError::into_inner)`.
pub struct StdRecovering;
/// parking_lot's locks.
pub struct ParkingLot;

/// A tokio runtime as the server starts one (`server.rs`): a worker a core.
fn runtime() -> tokio::runtime::Runtime {
    let cores = thread::available_parallelism().map_or(8, |n| n.get());
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cores)
        .enable_all()
        .build()
        .expect("a tokio runtime")
}

/// Spin, then yield, until `done`. The harness's threads wait for each
/// other so, rather than park: a futex's wake in every round would be the
/// harness's cost, not the channel's.
fn wait_until(done: impl Fn() -> bool) {
    let mut spins = 0u32;
    while !done() {
        if spins < 128 {
            spins += 1;
            std::hint::spin_loop();
        } else {
            thread::yield_now();
        }
    }
}

/// What a benchmark's consumers have taken, in all its rounds.
#[derive(Default)]
pub struct Tally(AtomicU64);

impl Tally {
    fn add(&self, n: usize) {
        self.0.fetch_add(n as u64, Ordering::Release);
    }
}

/// Rounds of `n` items each: [`Rounds::run`] starts one and returns once
/// its consumers have taken all `n`.
struct Rounds {
    tally: Arc<Tally>,
    n: u64,
    total: u64,
}

impl Rounds {
    fn new(n: usize) -> Self {
        Self {
            tally: Arc::default(),
            n: n as u64,
            total: 0,
        }
    }

    fn tally(&self) -> Arc<Tally> {
        self.tally.clone()
    }

    fn run(&mut self, start: impl FnOnce()) {
        self.total += self.n;
        start();
        let total = self.total;
        wait_until(|| self.tally.0.load(Ordering::Acquire) >= total);
    }
}

/// Plain threads that each run their part once a round, when the
/// benchmark's thread starts one ([`Crew::start`]).
struct Crew {
    round: Arc<AtomicU64>,
    threads: Vec<JoinHandle<()>>,
}

impl Crew {
    const STOP: u64 = u64::MAX;

    fn new(name: &str, parts: Vec<Box<dyn FnMut() + Send>>) -> Self {
        let round = Arc::new(AtomicU64::new(0));
        let threads = parts
            .into_iter()
            .enumerate()
            .map(|(i, mut part)| {
                let round = round.clone();
                thread::Builder::new()
                    .name(format!("{name} {i}"))
                    .spawn(move || {
                        let mut seen = 0;
                        loop {
                            wait_until(|| round.load(Ordering::Acquire) != seen);
                            seen = round.load(Ordering::Acquire);
                            if seen == Self::STOP {
                                return;
                            }
                            part();
                        }
                    })
                    .expect("a crew thread")
            })
            .collect();
        Self { round, threads }
    }

    fn start(&self) {
        self.round.fetch_add(1, Ordering::Release);
    }
}

impl Drop for Crew {
    fn drop(&mut self) {
        self.round.store(Self::STOP, Ordering::Release);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// wvs's channels, each on its own path's shape.
#[divan::bench_group(min_time = SECS, max_time = SECS)]
mod channels {
    use super::*;

    use std::sync::mpsc as std_mpsc;
    use tokio::sync::{mpsc, oneshot};
    use wow_vanilla_server::game::guid;
    use wow_vanilla_server::game::map::{MapEvent, UPDATE_INTERVAL};
    use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
    use wow_world_messages::vanilla::{
        MSG_MOVE_HEARTBEAT_Client, MovementInfo, MovementInfo_MovementFlags, Vector3d,
    };

    // ---- A client's outbound queue (`conn.rs`) ----

    /// A frame as `conn::encode` makes it: `[size][opcode][body]`, shared.
    pub type Frame = Arc<[u8]>;

    /// The frames in a round, whatever the producers, so that the rows
    /// compare.
    const FRAMES: usize = 16 * 1024;
    /// `run_writer`'s batch.
    const BATCH_LIMIT: usize = 128;
    /// What maps send, header in, in bytes: movement relays (36), monster
    /// moves, values updates and blows (48 to 96), a create or a chat line
    /// (192, 384). The channels move the 16-byte `Arc`; the size is the
    /// writer's copy.
    const FRAME_SIZES: [usize; 8] = [36, 36, 48, 56, 64, 96, 192, 384];

    /// The two ends of an outbound queue, as `conn.rs` uses them.
    pub trait OutboundQueue: 'static {
        type Tx: Clone + Send + 'static;
        type Rx: Send + 'static;
        fn channel() -> (Self::Tx, Self::Rx);
        /// `Outbound::send_frame_unheld`'s send: sync, from any thread.
        fn send(tx: &Self::Tx, frame: Frame);
        /// `run_writer`'s receive: wait for a frame, then take what else is
        /// queued, up to [`BATCH_LIMIT`]. False once every sender has gone.
        fn recv_batch(
            rx: &mut Self::Rx,
            staging: &mut Vec<Frame>,
        ) -> impl Future<Output = bool> + Send;
    }

    /// wvs before `chore/channels`: kanal's `unbounded_async`, sent to
    /// with `try_send`, drained with a sync clone's `try_recv`.
    impl OutboundQueue for Kanal {
        type Tx = kanal::AsyncSender<Frame>;
        type Rx = (kanal::AsyncReceiver<Frame>, kanal::Receiver<Frame>);
        fn channel() -> (Self::Tx, Self::Rx) {
            let (tx, rx) = kanal::unbounded_async();
            let sync = rx.clone_sync();
            (tx, (rx, sync))
        }
        fn send(tx: &Self::Tx, frame: Frame) {
            let _ = tx.try_send(frame);
        }
        async fn recv_batch((rx, sync): &mut Self::Rx, staging: &mut Vec<Frame>) -> bool {
            let Ok(first) = rx.recv().await else {
                return false;
            };
            staging.push(first);
            while staging.len() < BATCH_LIMIT {
                match sync.try_recv() {
                    Ok(Some(f)) => staging.push(f),
                    Ok(None) | Err(_) => break,
                }
            }
            true
        }
    }

    /// tokio's `unbounded_channel`, drained as `run_writer` drained kanal's.
    impl OutboundQueue for Tokio {
        type Tx = mpsc::UnboundedSender<Frame>;
        type Rx = mpsc::UnboundedReceiver<Frame>;
        fn channel() -> (Self::Tx, Self::Rx) {
            mpsc::unbounded_channel()
        }
        fn send(tx: &Self::Tx, frame: Frame) {
            let _ = tx.send(frame);
        }
        async fn recv_batch(rx: &mut Self::Rx, staging: &mut Vec<Frame>) -> bool {
            let Some(first) = rx.recv().await else {
                return false;
            };
            staging.push(first);
            while staging.len() < BATCH_LIMIT {
                match rx.try_recv() {
                    Ok(f) => staging.push(f),
                    Err(_) => break,
                }
            }
            true
        }
    }

    /// wvs today: tokio's `unbounded_channel`, drained with `recv_many`.
    impl OutboundQueue for TokioRecvMany {
        type Tx = mpsc::UnboundedSender<Frame>;
        type Rx = mpsc::UnboundedReceiver<Frame>;
        fn channel() -> (Self::Tx, Self::Rx) {
            mpsc::unbounded_channel()
        }
        fn send(tx: &Self::Tx, frame: Frame) {
            let _ = tx.send(frame);
        }
        async fn recv_batch(rx: &mut Self::Rx, staging: &mut Vec<Frame>) -> bool {
            rx.recv_many(staging, BATCH_LIMIT).await > 0
        }
    }

    /// flume's `unbounded`: `recv_async`, then `try_recv`.
    impl OutboundQueue for Flume {
        type Tx = flume::Sender<Frame>;
        type Rx = flume::Receiver<Frame>;
        fn channel() -> (Self::Tx, Self::Rx) {
            flume::unbounded()
        }
        fn send(tx: &Self::Tx, frame: Frame) {
            let _ = tx.send(frame);
        }
        async fn recv_batch(rx: &mut Self::Rx, staging: &mut Vec<Frame>) -> bool {
            let Ok(first) = rx.recv_async().await else {
                return false;
            };
            staging.push(first);
            while staging.len() < BATCH_LIMIT {
                match rx.try_recv() {
                    Ok(f) => staging.push(f),
                    Err(_) => break,
                }
            }
            true
        }
    }

    /// `run_writer` without the cipher and the socket: take a batch, copy
    /// its frames out, give their bytes back.
    async fn writer<C: OutboundQueue>(mut rx: C::Rx, tally: Arc<Tally>) {
        let mut staging = Vec::with_capacity(BATCH_LIMIT);
        let mut scratch: Vec<u8> = Vec::with_capacity(16 * 1024);
        while C::recv_batch(&mut rx, &mut staging).await {
            scratch.clear();
            for f in &staging {
                scratch.extend_from_slice(f);
            }
            black_box(&scratch);
            tally.add(staging.len());
            staging.clear();
        }
    }

    /// A client's outbound queue (`conn.rs`, `Outbound`): producers on
    /// plain threads (maps, the services) send frames with a sync send,
    /// one writer task on the runtime drains them (`run_writer`). A round
    /// is 16,384 frames, split between 1, 4 or 16 producers; each sends
    /// clones of its own frames, as a map shares one frame between its
    /// observers. Without the byte budget's semaphore, which is the same
    /// whatever the channel.
    #[divan::bench(types = [Kanal, Tokio, TokioRecvMany, Flume], args = [1, 4, 16])]
    fn outbound<C: OutboundQueue>(bencher: Bencher, producers: usize) {
        let rt = runtime();
        let (tx, rx) = C::channel();
        let mut rounds = Rounds::new(FRAMES);
        rt.spawn(writer::<C>(rx, rounds.tally()));
        let per = FRAMES / producers;
        let parts = (0..producers)
            .map(|_| {
                let tx = tx.clone();
                let frames: Vec<Frame> = FRAME_SIZES
                    .iter()
                    .map(|&n| Arc::from(vec![0x5Au8; n]))
                    .collect();
                Box::new(move || {
                    for k in 0..per {
                        C::send(&tx, frames[k % FRAME_SIZES.len()].clone());
                    }
                }) as Box<dyn FnMut() + Send>
            })
            .collect();
        drop(tx);
        let crew = Crew::new("producer", parts);
        bencher
            .counter(ItemsCount::new(FRAMES))
            .bench_local(|| rounds.run(|| crew.start()));
    }

    // ---- A map's mailbox (`map/mod.rs`) ----

    /// The events in a round, whatever the senders.
    const EVENTS: usize = 4096;
    /// What the map's loop takes after an event before it looks at its
    /// ticker again (`map::run`).
    const DRAIN: usize = 512;

    /// A map's mailbox: `MapHandle`'s sender, and the map's loop.
    pub trait Mailbox: 'static {
        type Tx: Clone + Send + Sync + 'static;
        type Rx: Send + 'static;
        fn channel() -> (Self::Tx, Self::Rx);
        /// `MapHandle::send`: sync, from a session's task.
        fn send(tx: &Self::Tx, event: MapEvent);
        /// The map's loop (`map::run`), counting the events it handles,
        /// until every sender has gone.
        fn run(rx: Self::Rx, tally: Arc<Tally>) -> impl Future<Output = ()> + Send;
    }

    /// A movement packet from a session, what a map is sent most: the real
    /// `MapEvent`, so each send moves what wvs's does.
    fn event(sender: usize, i: usize) -> MapEvent {
        MapEvent::Packet(
            guid::player(sender as u32 + 1),
            ClientOpcodeMessage::MSG_MOVE_HEARTBEAT(MSG_MOVE_HEARTBEAT_Client {
                info: MovementInfo {
                    flags: MovementInfo_MovementFlags::new_forward(),
                    timestamp: i as u32,
                    position: Vector3d {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                    orientation: 0.0,
                    fall_time: 0,
                },
            }),
        )
    }

    /// `handle_guarded`, for the benchmark: the event is taken and dropped.
    fn handle(event: MapEvent) {
        drop(black_box(event));
    }

    /// The map's 50 ms ticker, as `map::run` sets it.
    fn ticker() -> tokio::time::Interval {
        let mut ticker = tokio::time::interval(UPDATE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker
    }

    /// wvs today: tokio's `unbounded_channel`, received in `select!`
    /// beside the ticker, then drained with `try_recv`.
    impl Mailbox for Tokio {
        type Tx = mpsc::UnboundedSender<MapEvent>;
        type Rx = mpsc::UnboundedReceiver<MapEvent>;
        fn channel() -> (Self::Tx, Self::Rx) {
            mpsc::unbounded_channel()
        }
        fn send(tx: &Self::Tx, event: MapEvent) {
            let _ = tx.send(event);
        }
        async fn run(mut rx: Self::Rx, tally: Arc<Tally>) {
            let mut ticker = ticker();
            loop {
                tokio::select! {
                    event = rx.recv() => {
                        let Some(event) = event else { break };
                        handle(event);
                        let mut n = 1;
                        for _ in 0..DRAIN {
                            match rx.try_recv() {
                                Ok(event) => {
                                    handle(event);
                                    n += 1;
                                }
                                Err(_) => break,
                            }
                        }
                        tally.add(n);
                    }
                    _ = ticker.tick() => black_box(()),
                }
            }
        }
    }

    /// flume's `unbounded` in the same `select!`: its `recv_async` leaves
    /// a message in the queue when `select!` drops it, so it is safe there.
    impl Mailbox for Flume {
        type Tx = flume::Sender<MapEvent>;
        type Rx = flume::Receiver<MapEvent>;
        fn channel() -> (Self::Tx, Self::Rx) {
            flume::unbounded()
        }
        fn send(tx: &Self::Tx, event: MapEvent) {
            let _ = tx.send(event);
        }
        async fn run(rx: Self::Rx, tally: Arc<Tally>) {
            let mut ticker = ticker();
            loop {
                tokio::select! {
                    event = rx.recv_async() => {
                        let Ok(event) = event else { break };
                        handle(event);
                        let mut n = 1;
                        for _ in 0..DRAIN {
                            match rx.try_recv() {
                                Ok(event) => {
                                    handle(event);
                                    n += 1;
                                }
                                Err(_) => break,
                            }
                        }
                        tally.add(n);
                    }
                    _ = ticker.tick() => black_box(()),
                }
            }
        }
    }

    /// tokio's `unbounded_channel` in a loop of its own, no ticker: what
    /// `select!` costs, beside kanal's.
    impl Mailbox for TokioNoSelect {
        type Tx = mpsc::UnboundedSender<MapEvent>;
        type Rx = mpsc::UnboundedReceiver<MapEvent>;
        fn channel() -> (Self::Tx, Self::Rx) {
            mpsc::unbounded_channel()
        }
        fn send(tx: &Self::Tx, event: MapEvent) {
            let _ = tx.send(event);
        }
        async fn run(mut rx: Self::Rx, tally: Arc<Tally>) {
            while let Some(event) = rx.recv().await {
                handle(event);
                let mut n = 1;
                for _ in 0..DRAIN {
                    match rx.try_recv() {
                        Ok(event) => {
                            handle(event);
                            n += 1;
                        }
                        Err(_) => break,
                    }
                }
                tally.add(n);
            }
        }
    }

    /// kanal's `unbounded_async`, only in a loop of its own: its docs say
    /// that a `ReceiveFuture` dropped as a message arrives can lose it, and
    /// that `tokio::select!` isn't correct to use with it. A map on kanal
    /// would need its ticker elsewhere (a task sending it a tick).
    impl Mailbox for KanalNoSelect {
        type Tx = kanal::AsyncSender<MapEvent>;
        type Rx = kanal::AsyncReceiver<MapEvent>;
        fn channel() -> (Self::Tx, Self::Rx) {
            kanal::unbounded_async()
        }
        fn send(tx: &Self::Tx, event: MapEvent) {
            let _ = tx.try_send(event);
        }
        async fn run(rx: Self::Rx, tally: Arc<Tally>) {
            while let Ok(event) = rx.recv().await {
                handle(event);
                let mut n = 1;
                for _ in 0..DRAIN {
                    match rx.try_recv() {
                        Ok(Some(event)) => {
                            handle(event);
                            n += 1;
                        }
                        Ok(None) | Err(_) => break,
                    }
                }
                tally.add(n);
            }
        }
    }

    /// A map's mailbox (`map/mod.rs`, `MapHandle` and `map::run`): async
    /// senders (sessions, a task each, on the runtime's workers) send the
    /// real `MapEvent` with a sync send; the map's task receives in its
    /// loop, then drains up to 512. A round is 4,096 events, from 1 sender
    /// (a map with one player) or 64 (a busy continent).
    #[divan::bench(types = [Tokio, Flume, TokioNoSelect, KanalNoSelect], args = [1, 64])]
    fn mailbox<C: Mailbox>(bencher: Bencher, senders: usize) {
        let rt = runtime();
        let (tx, rx) = C::channel();
        let mut rounds = Rounds::new(EVENTS);
        rt.spawn(C::run(rx, rounds.tally()));
        let per = EVENTS / senders;
        bencher.counter(ItemsCount::new(EVENTS)).bench_local(|| {
            rounds.run(|| {
                let tx = tx.clone();
                // Spawned from a worker, as a session's task is woken there.
                rt.spawn(async move {
                    for s in 0..senders {
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            for i in 0..per {
                                C::send(&tx, event(s, i));
                            }
                        });
                    }
                });
            })
        });
    }

    // ---- The tile readers (`map/tiles.rs`) ----

    /// `tiles::READERS`.
    const READERS: usize = 4;
    /// The reads a round asks for: a player arriving asks for up to four
    /// navmesh tiles and four terrain tiles (`tiles::ahead`).
    const JOBS: usize = 8;
    /// How long the readers wait between rounds, untimed: long enough that
    /// they are asleep, as they are between a map's asks.
    const IDLE: std::time::Duration = std::time::Duration::from_millis(1);

    /// `tiles::read`'s job.
    pub type Job = Box<dyn FnOnce() + Send>;

    /// The readers' queue: one sender, [`READERS`] readers on plain
    /// threads, each taking the next job with a blocking receive.
    pub trait TileQueue: 'static {
        type Tx;
        type Rx: Send + 'static;
        fn channel() -> (Self::Tx, Self::Rx);
        fn clone_rx(rx: &Self::Rx) -> Self::Rx;
        fn send(tx: &Self::Tx, job: Job);
        /// A reader's blocking receive: `None` once the sender has gone.
        fn recv(rx: &Self::Rx) -> Option<Job>;
    }

    /// wvs before `chore/channels`: kanal's `unbounded`, MPMC.
    impl TileQueue for Kanal {
        type Tx = kanal::Sender<Job>;
        type Rx = kanal::Receiver<Job>;
        fn channel() -> (Self::Tx, Self::Rx) {
            kanal::unbounded()
        }
        fn clone_rx(rx: &Self::Rx) -> Self::Rx {
            rx.clone()
        }
        fn send(tx: &Self::Tx, job: Job) {
            let _ = tx.send(job);
        }
        fn recv(rx: &Self::Rx) -> Option<Job> {
            rx.recv().ok()
        }
    }

    /// crossbeam-channel's `unbounded`, MPMC.
    impl TileQueue for Crossbeam {
        type Tx = crossbeam_channel::Sender<Job>;
        type Rx = crossbeam_channel::Receiver<Job>;
        fn channel() -> (Self::Tx, Self::Rx) {
            crossbeam_channel::unbounded()
        }
        fn clone_rx(rx: &Self::Rx) -> Self::Rx {
            rx.clone()
        }
        fn send(tx: &Self::Tx, job: Job) {
            let _ = tx.send(job);
        }
        fn recv(rx: &Self::Rx) -> Option<Job> {
            rx.recv().ok()
        }
    }

    /// flume's `unbounded`, MPMC.
    impl TileQueue for Flume {
        type Tx = flume::Sender<Job>;
        type Rx = flume::Receiver<Job>;
        fn channel() -> (Self::Tx, Self::Rx) {
            flume::unbounded()
        }
        fn clone_rx(rx: &Self::Rx) -> Self::Rx {
            rx.clone()
        }
        fn send(tx: &Self::Tx, job: Job) {
            let _ = tx.send(job);
        }
        fn recv(rx: &Self::Rx) -> Option<Job> {
            rx.recv().ok()
        }
    }

    /// std's `mpsc` has one receiver: the readers share it behind std's
    /// `Mutex`, the one holding the lock waiting in `recv`.
    impl TileQueue for Std {
        type Tx = std_mpsc::Sender<Job>;
        type Rx = Arc<std::sync::Mutex<std_mpsc::Receiver<Job>>>;
        fn channel() -> (Self::Tx, Self::Rx) {
            let (tx, rx) = std_mpsc::channel();
            (tx, Arc::new(std::sync::Mutex::new(rx)))
        }
        fn clone_rx(rx: &Self::Rx) -> Self::Rx {
            rx.clone()
        }
        fn send(tx: &Self::Tx, job: Job) {
            let _ = tx.send(job);
        }
        fn recv(rx: &Self::Rx) -> Option<Job> {
            let rx = rx.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            rx.recv().ok()
        }
    }

    /// wvs today: tokio's `mpsc` has one receiver, so the readers share it
    /// behind a lock (parking_lot's, wvs's), the one holding it waiting in
    /// `blocking_recv`.
    impl TileQueue for Tokio {
        type Tx = mpsc::UnboundedSender<Job>;
        type Rx = Arc<parking_lot::Mutex<mpsc::UnboundedReceiver<Job>>>;
        fn channel() -> (Self::Tx, Self::Rx) {
            let (tx, rx) = mpsc::unbounded_channel();
            (tx, Arc::new(parking_lot::Mutex::new(rx)))
        }
        fn clone_rx(rx: &Self::Rx) -> Self::Rx {
            rx.clone()
        }
        fn send(tx: &Self::Tx, job: Job) {
            let _ = tx.send(job);
        }
        fn recv(rx: &Self::Rx) -> Option<Job> {
            rx.lock().blocking_recv()
        }
    }

    /// The tile readers (`map/tiles.rs`, `tiles::read`): a map sends boxed
    /// jobs to four reader threads, each waiting in a blocking receive. A
    /// round is 8 jobs (a player's arrival), sent to readers asleep; the
    /// jobs only count themselves, so the time is the channel's and the
    /// readers' waking, not the disk's.
    #[divan::bench(types = [Kanal, Crossbeam, Flume, Std, Tokio], sample_size = 1)]
    fn tiles<C: TileQueue>(bencher: Bencher) {
        let (tx, rx) = C::channel();
        let readers: Vec<JoinHandle<()>> = (0..READERS)
            .map(|i| {
                let rx = C::clone_rx(&rx);
                thread::Builder::new()
                    .name(format!("tiles {i}"))
                    .spawn(move || {
                        while let Some(job) = C::recv(&rx) {
                            job();
                        }
                    })
                    .expect("a reader thread")
            })
            .collect();
        drop(rx);
        let mut rounds = Rounds::new(JOBS);
        let tally = rounds.tally();
        bencher
            .counter(ItemsCount::new(JOBS))
            .with_inputs(|| thread::sleep(IDLE))
            .bench_local_values(|()| {
                rounds.run(|| {
                    for _ in 0..JOBS {
                        let tally = tally.clone();
                        C::send(&tx, Box::new(move || tally.add(1)));
                    }
                })
            });
        drop(tx);
        for r in readers {
            let _ = r.join();
        }
    }

    // ---- The store (`store.rs`) ----

    /// The round trips in a round, whatever the callers.
    const ROUND_TRIPS: usize = 256;
    /// What the store's thread takes at once (`serve_all`).
    const STORE_BATCH: usize = 512;

    /// A request to the store's thread, its reply by tokio's `oneshot` as
    /// `Store::load`'s.
    pub enum Request {
        Load(u32, oneshot::Sender<u64>),
        /// Never sent: wvs's largest request (`Request::Together`, two
        /// vectors), so that a slot is as big as wvs's.
        #[allow(dead_code)]
        Together(Vec<u64>, Vec<u64>),
    }

    /// The store's queue: async callers send, one plain thread receives.
    pub trait StoreQueue: 'static {
        type Tx: Clone + Send + 'static;
        type Rx: Send + 'static;
        fn channel() -> (Self::Tx, Self::Rx);
        /// `Store::send`: sync, from a session's task.
        fn send(tx: &Self::Tx, req: Request);
        /// The store thread's blocking receive: `None` once every sender
        /// has gone.
        fn recv(rx: &mut Self::Rx) -> Option<Request>;
        fn try_recv(rx: &mut Self::Rx) -> Option<Request>;
    }

    /// wvs before `chore/channels`: `std::sync::mpsc`.
    impl StoreQueue for Std {
        type Tx = std_mpsc::Sender<Request>;
        type Rx = std_mpsc::Receiver<Request>;
        fn channel() -> (Self::Tx, Self::Rx) {
            std_mpsc::channel()
        }
        fn send(tx: &Self::Tx, req: Request) {
            let _ = tx.send(req);
        }
        fn recv(rx: &mut Self::Rx) -> Option<Request> {
            rx.recv().ok()
        }
        fn try_recv(rx: &mut Self::Rx) -> Option<Request> {
            rx.try_recv().ok()
        }
    }

    /// wvs today: tokio's `unbounded_channel`, the thread receiving with
    /// `blocking_recv`.
    impl StoreQueue for Tokio {
        type Tx = mpsc::UnboundedSender<Request>;
        type Rx = mpsc::UnboundedReceiver<Request>;
        fn channel() -> (Self::Tx, Self::Rx) {
            mpsc::unbounded_channel()
        }
        fn send(tx: &Self::Tx, req: Request) {
            let _ = tx.send(req);
        }
        fn recv(rx: &mut Self::Rx) -> Option<Request> {
            rx.blocking_recv()
        }
        fn try_recv(rx: &mut Self::Rx) -> Option<Request> {
            rx.try_recv().ok()
        }
    }

    /// kanal's `unbounded`.
    impl StoreQueue for Kanal {
        type Tx = kanal::Sender<Request>;
        type Rx = kanal::Receiver<Request>;
        fn channel() -> (Self::Tx, Self::Rx) {
            kanal::unbounded()
        }
        fn send(tx: &Self::Tx, req: Request) {
            let _ = tx.send(req);
        }
        fn recv(rx: &mut Self::Rx) -> Option<Request> {
            rx.recv().ok()
        }
        fn try_recv(rx: &mut Self::Rx) -> Option<Request> {
            rx.try_recv().ok().flatten()
        }
    }

    /// flume's `unbounded`.
    impl StoreQueue for Flume {
        type Tx = flume::Sender<Request>;
        type Rx = flume::Receiver<Request>;
        fn channel() -> (Self::Tx, Self::Rx) {
            flume::unbounded()
        }
        fn send(tx: &Self::Tx, req: Request) {
            let _ = tx.send(req);
        }
        fn recv(rx: &mut Self::Rx) -> Option<Request> {
            rx.recv().ok()
        }
        fn try_recv(rx: &mut Self::Rx) -> Option<Request> {
            rx.try_recv().ok()
        }
    }

    /// crossbeam-channel's `unbounded`.
    impl StoreQueue for Crossbeam {
        type Tx = crossbeam_channel::Sender<Request>;
        type Rx = crossbeam_channel::Receiver<Request>;
        fn channel() -> (Self::Tx, Self::Rx) {
            crossbeam_channel::unbounded()
        }
        fn send(tx: &Self::Tx, req: Request) {
            let _ = tx.send(req);
        }
        fn recv(rx: &mut Self::Rx) -> Option<Request> {
            rx.recv().ok()
        }
        fn try_recv(rx: &mut Self::Rx) -> Option<Request> {
            rx.try_recv().ok()
        }
    }

    /// `serve_all` without SQLite: wait for a request, take what else is
    /// queued (up to 512), answer each in order.
    fn serve<C: StoreQueue>(mut rx: C::Rx) {
        while let Some(first) = C::recv(&mut rx) {
            let mut batch = vec![first];
            while let Some(more) = C::try_recv(&mut rx) {
                batch.push(more);
                if batch.len() >= STORE_BATCH {
                    break;
                }
            }
            for req in batch {
                if let Request::Load(guid, reply) = req {
                    let _ = reply.send(u64::from(guid));
                }
            }
        }
    }

    /// The store (`store.rs`): callers (sessions, a task each) send a
    /// request with a sync send and await its reply on tokio's `oneshot`;
    /// the store's plain thread waits in a blocking receive, takes what
    /// else is queued and answers. The reply is tokio's `oneshot` for every
    /// candidate, as wvs's is. A round is 256 round trips, one after
    /// another from 1 caller (a login) or 16 at once (a burst of logins).
    /// Without `Store::queued`, the count beside the channel, which is the
    /// same whatever the channel.
    #[divan::bench(types = [Std, Tokio, Kanal, Flume, Crossbeam], args = [1, 16])]
    fn store<C: StoreQueue>(bencher: Bencher, callers: usize) {
        let rt = runtime();
        let (tx, rx) = C::channel();
        let store = thread::Builder::new()
            .name("store".into())
            .spawn(move || serve::<C>(rx))
            .expect("the store's thread");
        let mut rounds = Rounds::new(ROUND_TRIPS);
        let tally = rounds.tally();
        let per = ROUND_TRIPS / callers;
        bencher
            .counter(ItemsCount::new(ROUND_TRIPS))
            .bench_local(|| {
                rounds.run(|| {
                    let tx = tx.clone();
                    let tally = tally.clone();
                    rt.spawn(async move {
                        for _ in 0..callers {
                            let tx = tx.clone();
                            let tally = tally.clone();
                            tokio::spawn(async move {
                                for i in 0..per {
                                    let (reply, answer) = oneshot::channel();
                                    C::send(&tx, Request::Load(i as u32, reply));
                                    black_box(answer.await.ok());
                                }
                                tally.add(per);
                            });
                        }
                    });
                })
            });
        drop(tx);
        let _ = store.join();
    }
}

/// The shared state's locks: parking_lot's, std's, and std's taken
/// whatever a panic left (as wvs would need to, if it moved to std's: a
/// poisoned registry would otherwise fail every lookup after it).
#[divan::bench_group(min_time = SECS, max_time = SECS, sample_size = OPS_A_SAMPLE)]
mod locks {
    use super::*;

    use std::cell::Cell;
    use std::sync::PoisonError;
    use std::sync::atomic::AtomicU8;

    use ahash::{AHashMap, AHashSet};
    use wow_vanilla_server::game::guid;
    use wow_world_messages::Guid;

    /// Players online: the registry's online map, and the friends lists.
    const ONLINE: usize = 1000;
    /// A registry write (a login: a player in and out) every this many
    /// operations on each thread.
    const LOGIN_EVERY: u64 = 1000;
    /// The characters a player's friends are among.
    const FRIENDS: u64 = 50;

    /// A lock kind, with closures so that each candidate's guard (and its
    /// poison check) is its own.
    pub trait Locks: 'static {
        type RwLock<T: Send + Sync>: Send + Sync;
        type Mutex<T: Send>: Send + Sync;
        fn rwlock<T: Send + Sync>(v: T) -> Self::RwLock<T>;
        fn read<T: Send + Sync, R>(l: &Self::RwLock<T>, f: impl FnOnce(&T) -> R) -> R;
        fn write<T: Send + Sync, R>(l: &Self::RwLock<T>, f: impl FnOnce(&mut T) -> R) -> R;
        fn mutex<T: Send>(v: T) -> Self::Mutex<T>;
        fn lock<T: Send, R>(m: &Self::Mutex<T>, f: impl FnOnce(&mut T) -> R) -> R;
    }

    /// wvs today.
    impl Locks for ParkingLot {
        type RwLock<T: Send + Sync> = parking_lot::RwLock<T>;
        type Mutex<T: Send> = parking_lot::Mutex<T>;
        fn rwlock<T: Send + Sync>(v: T) -> Self::RwLock<T> {
            parking_lot::RwLock::new(v)
        }
        fn read<T: Send + Sync, R>(l: &Self::RwLock<T>, f: impl FnOnce(&T) -> R) -> R {
            f(&l.read())
        }
        fn write<T: Send + Sync, R>(l: &Self::RwLock<T>, f: impl FnOnce(&mut T) -> R) -> R {
            f(&mut l.write())
        }
        fn mutex<T: Send>(v: T) -> Self::Mutex<T> {
            parking_lot::Mutex::new(v)
        }
        fn lock<T: Send, R>(m: &Self::Mutex<T>, f: impl FnOnce(&mut T) -> R) -> R {
            f(&mut m.lock())
        }
    }

    /// std's, `unwrap`ped: a poisoned lock panics.
    impl Locks for Std {
        type RwLock<T: Send + Sync> = std::sync::RwLock<T>;
        type Mutex<T: Send> = std::sync::Mutex<T>;
        fn rwlock<T: Send + Sync>(v: T) -> Self::RwLock<T> {
            std::sync::RwLock::new(v)
        }
        fn read<T: Send + Sync, R>(l: &Self::RwLock<T>, f: impl FnOnce(&T) -> R) -> R {
            f(&l.read().unwrap())
        }
        fn write<T: Send + Sync, R>(l: &Self::RwLock<T>, f: impl FnOnce(&mut T) -> R) -> R {
            f(&mut l.write().unwrap())
        }
        fn mutex<T: Send>(v: T) -> Self::Mutex<T> {
            std::sync::Mutex::new(v)
        }
        fn lock<T: Send, R>(m: &Self::Mutex<T>, f: impl FnOnce(&mut T) -> R) -> R {
            f(&mut m.lock().unwrap())
        }
    }

    /// std's, a poisoned lock taken anyway.
    impl Locks for StdRecovering {
        type RwLock<T: Send + Sync> = std::sync::RwLock<T>;
        type Mutex<T: Send> = std::sync::Mutex<T>;
        fn rwlock<T: Send + Sync>(v: T) -> Self::RwLock<T> {
            std::sync::RwLock::new(v)
        }
        fn read<T: Send + Sync, R>(l: &Self::RwLock<T>, f: impl FnOnce(&T) -> R) -> R {
            f(&l.read().unwrap_or_else(PoisonError::into_inner))
        }
        fn write<T: Send + Sync, R>(l: &Self::RwLock<T>, f: impl FnOnce(&mut T) -> R) -> R {
            f(&mut l.write().unwrap_or_else(PoisonError::into_inner))
        }
        fn mutex<T: Send>(v: T) -> Self::Mutex<T> {
            std::sync::Mutex::new(v)
        }
        fn lock<T: Send, R>(m: &Self::Mutex<T>, f: impl FnOnce(&mut T) -> R) -> R {
            f(&mut m.lock().unwrap_or_else(PoisonError::into_inner))
        }
    }

    static SEEDS: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);

    thread_local! {
        /// This thread's operations so far, and its xorshift state.
        static OPS: Cell<(u64, u64)> = Cell::new((
            0,
            SEEDS.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed) | 1,
        ));
    }

    /// This thread's next operation: its number, and a random number.
    fn next_op() -> (u64, u64) {
        OPS.with(|ops| {
            let (n, mut x) = ops.get();
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            ops.set((n + 1, x));
            (n + 1, x)
        })
    }

    /// A stand-in for `registry::Online`: a lookup clones its `Arc`.
    pub struct Online {
        level: AtomicU8,
    }

    /// The registry (`registry.rs`): `RwLock<AHashMap<Guid, Arc<Online>>>`
    /// of 1,000 players, looked up by GUID (`Registry::online`: a read
    /// lock, the `Arc` cloned, a field read, the `Arc` dropped) from every
    /// thread, read-mostly; every thousandth operation on a thread is a
    /// write (a login: a player out and in under the write lock).
    #[divan::bench(types = [ParkingLot, Std, StdRecovering], threads = [1, 4, 16, 32])]
    fn registry<L: Locks>(bencher: Bencher) {
        let guids: Vec<Guid> = (1..=ONLINE as u32).map(guid::player).collect();
        let online = L::rwlock(
            guids
                .iter()
                .map(|&g| {
                    let o = Online {
                        level: AtomicU8::new(60),
                    };
                    (g, Arc::new(o))
                })
                .collect::<AHashMap<Guid, Arc<Online>>>(),
        );
        bencher.bench(|| {
            let (n, r) = next_op();
            let g = guids[(r % ONLINE as u64) as usize];
            if n % LOGIN_EVERY == 0 {
                L::write(&online, |m| {
                    if let Some(o) = m.remove(&g) {
                        m.insert(g, o);
                    }
                });
                0
            } else {
                L::read(&online, |m| m.get(&g).cloned())
                    .map_or(0, |o| o.level.load(Ordering::Relaxed))
            }
        });
    }

    /// A stand-in for `friends::Lists`: each player's friends.
    #[derive(Default)]
    pub struct Lists {
        friends: AHashMap<Guid, AHashSet<Guid>>,
    }

    /// The social state (`social/mod.rs`): a service's `Mutex` (here the
    /// friends lists of 1,000 players), taken for a short update (a friend
    /// added or removed) from every thread at once.
    #[divan::bench(types = [ParkingLot, Std, StdRecovering], threads = [1, 4, 16])]
    fn social<L: Locks>(bencher: Bencher) {
        let mut lists = Lists::default();
        for owner in 1..=ONLINE as u32 {
            let friends = (1..=FRIENDS as u32 / 2).map(|f| guid::player(f * 2));
            lists.friends.insert(guid::player(owner), friends.collect());
        }
        let lists = L::mutex(lists);
        bencher.bench(|| {
            let (_, r) = next_op();
            let owner = guid::player((r % ONLINE as u64) as u32 + 1);
            let friend = guid::player(((r >> 32) % FRIENDS) as u32 + 1);
            L::lock(&lists, |l| {
                let set = l.friends.entry(owner).or_default();
                if !set.remove(&friend) {
                    set.insert(friend);
                }
                set.len()
            })
        });
    }
}

/// The live settings (`game/mod.rs`, `Realm::config`): the real
/// `RealmConfig` behind each way of reading it, from every thread at once.
/// A map reads the reload counter once an update and takes the settings
/// only after a reload; sessions and services call `Realm::config`.
#[divan::bench_group(min_time = SECS, max_time = SECS, sample_size = OPS_A_SAMPLE)]
mod live_config {
    use super::*;

    use arc_swap::ArcSwap;
    use wow_vanilla_server::game::RealmConfig;

    /// The settings, each way.
    struct Live {
        generation: AtomicU64,
        parking_lot: parking_lot::RwLock<Arc<RealmConfig>>,
        std: std::sync::RwLock<Arc<RealmConfig>>,
        swap: ArcSwap<RealmConfig>,
    }

    fn live() -> Live {
        let config = Arc::new(RealmConfig::default());
        Live {
            generation: AtomicU64::new(0),
            parking_lot: parking_lot::RwLock::new(config.clone()),
            std: std::sync::RwLock::new(config.clone()),
            swap: ArcSwap::new(config),
        }
    }

    /// `Realm::config_generation`, a map's look once an update: an
    /// acquire load.
    #[divan::bench(threads = [1, 4, 16, 32])]
    fn generation(bencher: Bencher) {
        let live = live();
        bencher.bench(|| live.generation.load(Ordering::Acquire));
    }

    /// `Realm::config` today: parking_lot's read lock and the `Arc` cloned,
    /// a field read, the clone dropped.
    #[divan::bench(threads = [1, 4, 16, 32])]
    fn parking_lot_read_clone(bencher: Bencher) {
        let live = live();
        bencher.bench(|| {
            let config = live.parking_lot.read().clone();
            config.visibility_yards
        });
    }

    /// The same through std's `RwLock`.
    #[divan::bench(threads = [1, 4, 16, 32])]
    fn std_read_clone(bencher: Bencher) {
        let live = live();
        bencher.bench(|| {
            let config = live
                .std
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            config.visibility_yards
        });
    }

    /// arc-swap's `load`, `Realm::config` before parking_lot's lock: a
    /// guard, no clone, a field read.
    #[divan::bench(threads = [1, 4, 16, 32])]
    fn arc_swap_load(bencher: Bencher) {
        let live = live();
        bencher.bench(|| live.swap.load().visibility_yards);
    }

    /// arc-swap's `load_full`, `Realm::config_arc` before: an `Arc` to
    /// keep, as `Realm::config` returns now, a field read, the clone
    /// dropped.
    #[divan::bench(threads = [1, 4, 16, 32])]
    fn arc_swap_load_full(bencher: Bencher) {
        let live = live();
        bencher.bench(|| {
            let config = live.swap.load_full();
            config.visibility_yards
        });
    }
}
