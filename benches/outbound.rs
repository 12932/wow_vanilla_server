//! The outbound path (`conn.rs`): what a packet costs from the map's thread
//! to the client's socket.
//!
//! ```sh
//! cargo bench --bench outbound                  # everything
//! cargo bench --bench outbound -- writer        # a group: writer, fanout or delivered
//! cargo bench --bench outbound -- --max-time 0.5
//! WVS_BENCH_WORKERS=4 cargo bench --bench outbound -- delivered
//! ```
//!
//! - `writer`: one client's writer task (`run_writer`, its cipher and its
//!   write), its queue filled first, to a sink (no system call) or to a
//!   loopback socket; the frames taken singly (a write each, as when they
//!   trickle in) or as they come. `item/s` is packets written a second.
//! - `fanout`: the map's side: movement packets (a heartbeat's size) from
//!   `MOVERS` movers each to `n` observers, the queues emptied between rounds,
//!   untimed, sent one by one (`send_frame`), corked (`conn::Cork`, a push
//!   per observer at the end), or broadcast (`conn::broadcast`, as the map
//!   sends now: each packet kept once with its set). `item/s` is deliveries
//!   a second, on the map's thread.
//! - `lookups`: `fanout`'s corked sends with the map's own lookups around
//!   them (a GUID set, the players' table).
//! - `delivered`: end to end: the bench's thread (the map) sends the round's
//!   packets to `n` clients, each with a writer task on the tokio runtime
//!   and a loopback socket read on threads of their own; the round ends when
//!   every byte is read. `item/s` is deliveries a second.

use std::hint::black_box;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use divan::Bencher;
use divan::counter::ItemsCount;
use wow_srp::normalized_string::NormalizedString;
use wow_srp::vanilla_header::{EncrypterHalf, ProofSeed};
use wow_vanilla_server::game::conn::{self, Frames, Outbound};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    divan::main();
}

/// A heartbeat's frame to an observer: `MSG_MOVE_HEARTBEAT`, a packed GUID
/// and the movement info, about 36 bytes.
fn heartbeat() -> Arc<[u8]> {
    wow_vanilla_server::game::update::frame(0xEE, &[7u8; 32])
}

/// The server's half of a session's header cipher.
fn encrypter() -> EncrypterHalf {
    let username = NormalizedString::new("bench".to_string()).unwrap();
    let server_seed = ProofSeed::new();
    let server_value = server_seed.seed();
    let client_seed = ProofSeed::new();
    let client_value = client_seed.seed();
    let (proof, _client) = client_seed.into_client_header_crypto(&username, [0; 40], server_value);
    let server = server_seed
        .into_server_header_crypto(&username, [0; 40], proof, client_value)
        .unwrap();
    server.split().0
}

/// A stall bound no bench reaches.
const LONG: Duration = Duration::from_secs(600);

fn current_thread() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn runtime() -> tokio::runtime::Runtime {
    let workers = std::env::var("WVS_BENCH_WORKERS")
        .ok()
        .and_then(|w| w.parse().ok())
        .unwrap_or(4);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .unwrap()
}

/// A loopback pair: the server's end for tokio, the client's read to the
/// end on a thread of its own, counting into `read`.
fn socket(rt: &tokio::runtime::Runtime, read: Arc<AtomicU64>) -> tokio::net::TcpStream {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap();
    let client = std::net::TcpStream::connect(at).unwrap();
    let (server, _) = listener.accept().unwrap();
    server.set_nonblocking(true).unwrap();
    server.set_nodelay(true).unwrap();
    thread::spawn(move || {
        let mut client = client;
        let mut buf = vec![0u8; 256 * 1024];
        while let Ok(n) = client.read(&mut buf) {
            if n == 0 {
                break;
            }
            read.fetch_add(n as u64, Ordering::Release);
        }
    });
    let _g = rt.enter();
    tokio::net::TcpStream::from_std(server).unwrap()
}

/// One writer, its queue filled first.
#[divan::bench_group(min_time = 2.0, max_time = 3.0)]
mod writer {
    use super::*;

    const FRAMES: usize = 20_000;

    /// Every frame queued, the writer started: it takes them as it can.
    #[divan::bench]
    fn sink_as_they_come(bencher: Bencher) {
        let rt = current_thread();
        bencher
            .counter(ItemsCount::new(FRAMES))
            .with_inputs(|| {
                let (out, frames) = Outbound::with_budget(1 << 30);
                let f = heartbeat();
                for _ in 0..FRAMES {
                    out.send_frame(f.clone());
                }
                (out, frames, encrypter())
            })
            .bench_local_values(|(out, frames, enc): (Outbound, Frames, EncrypterHalf)| {
                drop(out);
                rt.block_on(conn::run_writer(tokio::io::sink(), enc, frames, LONG));
            });
    }

    /// Every frame a broadcast of the cork's, the client one of 3,000 in
    /// each: the writer takes its share of the cork's log.
    #[divan::bench]
    fn sink_broadcast(bencher: Bencher) {
        let rt = current_thread();
        let all = Arc::new(all_of(3000));
        bencher
            .counter(ItemsCount::new(FRAMES))
            .with_inputs(|| {
                let (out, frames) = Outbound::with_budget(1 << 30);
                let f = heartbeat();
                {
                    let _cork = conn::Cork::new();
                    for _ in 0..FRAMES {
                        conn::broadcast(1, &f, &all, |i| (i == 0).then(|| out.clone()));
                    }
                }
                (out, frames, encrypter())
            })
            .bench_local_values(|(out, frames, enc): (Outbound, Frames, EncrypterHalf)| {
                drop(out);
                rt.block_on(conn::run_writer(tokio::io::sink(), enc, frames, LONG));
            });
    }

    /// A frame at a time, each written before the next is sent: a write a
    /// packet, as when packets trickle in.
    #[divan::bench]
    fn sink_singly(bencher: Bencher) {
        let rt = current_thread();
        let (out, frames) = Outbound::with_budget(1 << 30);
        let writer = rt.spawn(conn::run_writer(
            tokio::io::sink(),
            encrypter(),
            frames,
            LONG,
        ));
        let f = heartbeat();
        bencher.counter(ItemsCount::new(1000usize)).bench_local(|| {
            rt.block_on(async {
                for _ in 0..1000 {
                    out.send_frame(f.clone());
                    while out.queued_bytes() > 0 {
                        tokio::task::yield_now().await;
                    }
                }
            });
        });
        drop(out);
        let _ = rt.block_on(writer);
    }

    /// A loopback socket, frames a few at a time (8), each burst written
    /// before the next: a system call each.
    #[divan::bench(args = [1, 8, 64])]
    fn socket_bursts(bencher: Bencher, burst: usize) {
        let rt = runtime();
        let read = Arc::new(AtomicU64::new(0));
        let stream = socket(&rt, read.clone());
        let (out, frames) = Outbound::with_budget(1 << 30);
        let writer = rt.spawn(conn::run_writer(stream, encrypter(), frames, LONG));
        let f = heartbeat();
        let rounds = 4096 / burst;
        bencher
            .counter(ItemsCount::new(rounds * burst))
            .bench_local(|| {
                for _ in 0..rounds {
                    for _ in 0..burst {
                        out.send_frame(f.clone());
                    }
                    while out.queued_bytes() > 0 {
                        std::hint::spin_loop();
                    }
                }
            });
        out.close();
        let _ = rt.block_on(writer);
        black_box(read.load(Ordering::Acquire));
    }
}

/// Every one of `n` clients.
fn all_of(n: usize) -> conn::Clients {
    let mut c = conn::Clients::default();
    for i in 0..n {
        c.insert(i);
    }
    c
}

/// Movers a round in `fanout` and `delivered`.
const MOVERS: usize = 64;

/// The map's side: each mover's heartbeat to every observer.
#[divan::bench_group(min_time = 2.0, max_time = 3.0)]
mod fanout {
    use super::*;

    fn queues(n: usize) -> (Vec<Outbound>, Vec<Frames>) {
        (0..n).map(|_| Outbound::with_budget(1 << 30)).unzip()
    }

    fn empty(frames: &mut [Frames]) {
        for f in frames {
            while f.try_recv().is_some() {}
        }
    }

    /// `send_frame` to each observer, as `Map::send_to_observers` does.
    #[divan::bench(args = [100, 880, 2000, 3000])]
    fn direct(bencher: Bencher, n: usize) {
        let (outs, mut frames) = queues(n);
        let movers: Vec<Arc<[u8]>> = (0..MOVERS).map(|_| heartbeat()).collect();
        bencher
            .counter(ItemsCount::new(MOVERS * n))
            .with_inputs(|| empty(&mut frames))
            .bench_local_values(|()| {
                for m in &movers {
                    for o in &outs {
                        o.send_frame(m.clone());
                    }
                }
            });
    }

    /// Corked, as `Map::send_to_observers` sends now: each packet kept
    /// once with its set of observers (`conn::broadcast`), each observer's
    /// writer taking its share; a push per observer at the end, the
    /// budget counted then.
    #[divan::bench(args = [100, 880, 2000, 3000])]
    fn broadcast(bencher: Bencher, n: usize) {
        let (outs, mut frames) = queues(n);
        let movers: Vec<Arc<[u8]>> = (0..MOVERS).map(|_| heartbeat()).collect();
        let all = Arc::new(all_of(n));
        bencher
            .counter(ItemsCount::new(MOVERS * n))
            .with_inputs(|| empty(&mut frames))
            .bench_local_values(|()| {
                let _cork = conn::Cork::new();
                for m in &movers {
                    conn::broadcast(1, m, &all, |i| outs.get(i).cloned());
                }
            });
    }

    /// The same, corked: a push per observer at the end.
    #[divan::bench(args = [100, 880, 2000, 3000])]
    fn corked(bencher: Bencher, n: usize) {
        let (outs, mut frames) = queues(n);
        let movers: Vec<Arc<[u8]>> = (0..MOVERS).map(|_| heartbeat()).collect();
        bencher
            .counter(ItemsCount::new(MOVERS * n))
            .with_inputs(|| empty(&mut frames))
            .bench_local_values(|()| {
                let _cork = conn::Cork::new();
                for m in &movers {
                    for o in &outs {
                        o.send_frame(m.clone());
                    }
                }
            });
    }
}

/// The map's own lookups around the push: each observer a GUID in the
/// mover's set, looked up in the map's players (a player is about 5 KB, so
/// each lookup is a cache line of its own) or in a table of their queues
/// alone; corked, as the map sends.
#[divan::bench_group(min_time = 2.0, max_time = 3.0)]
mod lookups {
    use super::*;
    use ahash::{AHashMap, AHashSet};

    /// A player's size, as the map holds them.
    struct Player {
        _before: [u64; 300],
        outbound: Outbound,
        _after: [u64; 370],
    }

    struct World {
        players: AHashMap<u64, Player>,
        outbounds: AHashMap<u64, Outbound>,
        observers: AHashSet<u64>,
        frames: Vec<Frames>,
    }

    fn world(n: usize) -> World {
        let mut w = World {
            players: AHashMap::new(),
            outbounds: AHashMap::new(),
            observers: AHashSet::new(),
            frames: Vec::new(),
        };
        for i in 0..n as u64 {
            let (out, f) = Outbound::with_budget(1 << 30);
            w.players.insert(
                i + 1,
                Player {
                    _before: [i; 300],
                    outbound: out.clone(),
                    _after: [i; 370],
                },
            );
            w.outbounds.insert(i + 1, out);
            w.observers.insert(i + 1);
            w.frames.push(f);
        }
        w
    }

    fn empty(frames: &mut [Frames]) {
        for f in frames {
            while f.try_recv().is_some() {}
        }
    }

    #[divan::bench(args = [880, 2000, 3000])]
    fn through_players(bencher: Bencher, n: usize) {
        let mut w = world(n);
        let movers: Vec<Arc<[u8]>> = (0..MOVERS).map(|_| heartbeat()).collect();
        bencher
            .counter(ItemsCount::new(MOVERS * n))
            .with_inputs(|| empty(&mut w.frames))
            .bench_local_values(|()| {
                let _cork = conn::Cork::new();
                for m in &movers {
                    for o in &w.observers {
                        if let Some(p) = w.players.get(o) {
                            p.outbound.send_frame(m.clone());
                        }
                    }
                }
            });
    }

    #[divan::bench(args = [880, 2000, 3000])]
    fn through_outbounds(bencher: Bencher, n: usize) {
        let mut w = world(n);
        let movers: Vec<Arc<[u8]>> = (0..MOVERS).map(|_| heartbeat()).collect();
        bencher
            .counter(ItemsCount::new(MOVERS * n))
            .with_inputs(|| empty(&mut w.frames))
            .bench_local_values(|()| {
                let _cork = conn::Cork::new();
                for m in &movers {
                    for o in &w.observers {
                        if let Some(out) = w.outbounds.get(o) {
                            out.send_frame(m.clone());
                        }
                    }
                }
            });
    }
}

/// End to end, to loopback sockets.
#[divan::bench_group(min_time = 3.0, max_time = 4.0, sample_count = 30)]
mod delivered {
    use super::*;

    struct Clients {
        rt: tokio::runtime::Runtime,
        outs: Vec<Outbound>,
        read: Vec<Arc<AtomicU64>>,
        rounds: u64,
    }

    impl Clients {
        fn new(n: usize) -> Self {
            let rt = runtime();
            let mut outs = Vec::new();
            let mut read = Vec::new();
            for _ in 0..n {
                let r = Arc::new(AtomicU64::new(0));
                let stream = socket(&rt, r.clone());
                let (out, frames) = Outbound::with_budget(1 << 30);
                rt.spawn(conn::run_writer(stream, encrypter(), frames, LONG));
                outs.push(out);
                read.push(r);
            }
            Self {
                rt,
                outs,
                read,
                rounds: 0,
            }
        }

        fn wait(&self, bytes_each: u64) {
            for r in &self.read {
                while r.load(Ordering::Acquire) < bytes_each {
                    std::hint::spin_loop();
                }
            }
        }
    }

    impl Drop for Clients {
        fn drop(&mut self) {
            for o in &self.outs {
                o.close();
            }
            let _ = &self.rt;
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum How {
        Direct,
        Corked,
        Broadcast,
    }

    fn round(bencher: Bencher, n: usize, how: How) {
        let mut clients = Clients::new(n);
        let movers: Vec<Arc<[u8]>> = (0..MOVERS).map(|_| heartbeat()).collect();
        let per_round = (MOVERS * heartbeat().len()) as u64;
        let all = Arc::new(all_of(n));
        bencher
            .counter(ItemsCount::new(MOVERS * n))
            .bench_local(|| {
                clients.rounds += 1;
                {
                    let _cork = (how != How::Direct).then(conn::Cork::new);
                    for m in &movers {
                        if how == How::Broadcast {
                            conn::broadcast(1, m, &all, |i| clients.outs.get(i).cloned());
                        } else {
                            for o in &clients.outs {
                                o.send_frame(m.clone());
                            }
                        }
                    }
                }
                clients.wait(clients.rounds * per_round);
            });
    }

    #[divan::bench(args = [100, 880, 2000])]
    fn direct(bencher: Bencher, n: usize) {
        round(bencher, n, How::Direct);
    }

    #[divan::bench(args = [100, 880, 2000])]
    fn corked(bencher: Bencher, n: usize) {
        round(bencher, n, How::Corked);
    }

    #[divan::bench(args = [100, 880, 2000, 3000])]
    fn broadcast(bencher: Bencher, n: usize) {
        round(bencher, n, How::Broadcast);
    }
}
