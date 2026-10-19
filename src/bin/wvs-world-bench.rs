//! The whole world awake: the server alone, in one process, with observers
//! all over Eastern Kingdoms and Kalimdor (docs/PERFORMANCE.md, "The whole
//! world awake").
//!
//! It loads the real world as the server does, starts both continents'
//! actors, and logs in N players through the real session over channels (as
//! the tests' harness does: no sockets, no encryption), each sent with `.go`
//! to one of the first N points of a points file (`tools/world_points.py`).
//! Then it records every update of both continents for a fixed time: wall
//! and CPU time, the steps' CPU time, the creatures awake, and the CPU time
//! of the events between updates; and the process's CPU and peak memory.
//!
//! ```sh
//! cargo build --release --features world-bench --bin wvs-world-bench
//! target/release/wvs-world-bench --config config.toml --points docs/world_points.csv \
//!     --fraction 0.25 --secs 60 [--walk]
//! ```
//!
//! Only with the `world-bench` feature: the normal build and the tests
//! don't build it, and without the feature the maps carry no probe.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ahash::AHashMap;
use clap::Parser;
use tokio::sync::mpsc;
use wow_vanilla_server::game::conn::{Connection, Inbound, Outbound};
use wow_vanilla_server::game::map::MapKey;
use wow_vanilla_server::game::map::bench_probe::{self, Sample};
use wow_vanilla_server::game::store::Store;
use wow_vanilla_server::game::{Realm, RealmConfig, data, session};
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::*;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Observers all over both continents, and the maps' updates measured.
#[derive(Parser)]
#[command(name = "wvs-world-bench")]
struct Args {
    /// The server's config: `[world] data_dir`, `nav_dir` and
    /// `terrain_dir` say where the world is.
    #[arg(long, default_value = "config.toml")]
    config: PathBuf,
    /// The points: `map,x,y,z` a line (a header line is skipped).
    #[arg(long, default_value = "docs/world_points.csv")]
    points: PathBuf,
    /// The share of the points to put an observer at, from the file's start.
    #[arg(long, default_value_t = 1.0)]
    fraction: f64,
    /// Or this many observers (over --fraction).
    #[arg(long)]
    count: Option<usize>,
    /// How long to measure, in seconds.
    #[arg(long, default_value_t = 60)]
    secs: u64,
    /// Seconds after every observer is in place before measuring.
    #[arg(long, default_value_t = 10)]
    settle: u64,
    /// The observers walk at random, at running speed, from their points.
    #[arg(long)]
    walk: bool,
    /// The observers are ordinary players, not in GM mode: creatures
    /// hostile to them attack. (In GM mode creatures look for them, and
    /// don't attack.)
    #[arg(long)]
    no_gm: bool,
    /// Logins under way at once.
    #[arg(long, default_value_t = 64)]
    logins: usize,
    /// Write the results as JSON here too.
    #[arg(long)]
    json: Option<PathBuf>,
}

/// One observer, once it is in place.
struct Placed {
    tx: mpsc::Sender<Inbound>,
    at: Vector3d,
    heading: f32,
}

/// What the observers were sent, and their queues' bytes.
static BYTES_IN: AtomicU64 = AtomicU64::new(0);
static FRAMES_IN: AtomicU64 = AtomicU64::new(0);

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cores)
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(run(args))
}

fn read_points(path: &std::path::Path) -> Vec<(u32, Vector3d)> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("can't read {}: {e}", path.display()));
    text.lines()
        .filter_map(|l| {
            let mut f = l.split(',').map(str::trim);
            let map = f.next()?.parse().ok()?;
            let x = f.next()?.parse().ok()?;
            let y = f.next()?.parse().ok()?;
            let z = f.next()?.parse().ok()?;
            Some((map, Vector3d { x, y, z }))
        })
        .collect()
}

/// A name from the observer's number: letters only, as names must be.
fn name_of(i: usize) -> String {
    let mut s = String::from("Wb");
    let mut n = i;
    for _ in 0..6 {
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    s
}

async fn run(args: Args) -> std::process::ExitCode {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                // The update's over-50-ms warning would be a line an update.
                .with_filter(wow_vanilla_server::server::log_filter(
                    "warn,wow_vanilla_server::game::map=error,landmark=off",
                )),
        )
        .init();
    let cfg = match wow_vanilla_server::config::load(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 2.into();
        }
    };
    let cfg = wow_vanilla_server::config::install(cfg);
    let Some(data_dir) = cfg.world.data_dir.clone() else {
        eprintln!("the config has no [world] data_dir");
        return 2.into();
    };
    let points = read_points(&args.points);
    let n = args
        .count
        .unwrap_or_else(|| (points.len() as f64 * args.fraction).round() as usize)
        .min(points.len());
    let points: Vec<_> = points.into_iter().take(n).collect();
    let loadavg = || std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    println!(
        "{n} observers ({} on map 0, {} on map 1){}{}; load average {}",
        points.iter().filter(|p| p.0 == 0).count(),
        points.iter().filter(|p| p.0 == 1).count(),
        if args.walk { ", walking" } else { "" },
        if args.no_gm { ", not in GM mode" } else { "" },
        loadavg().trim()
    );

    let t = Instant::now();
    let world = data::load::load(&data_dir);
    println!("the world loaded in {:.1} s", t.elapsed().as_secs_f64());
    let (store, startup) = Store::open(":memory:").expect("an in-memory store");
    let mut config = RealmConfig::from_config(cfg);
    config.everyone_gm = true;
    let realm = Realm::new(config, store, startup, world, None);
    let t = Instant::now();
    for m in [0, 1] {
        let h = realm.maps.get(&realm, MapKey::world(m));
        // Answered once the map has populated.
        h.query(|_| ()).await;
    }
    println!(
        "the continents populated in {:.1} s",
        t.elapsed().as_secs_f64()
    );

    // In: every observer logs in and goes to its point. The probe is on
    // from here, for the slow events and updates it reports; what it
    // records before the measurement is dropped.
    bench_probe::enable();
    let t = Instant::now();
    let gate = Arc::new(tokio::sync::Semaphore::new(args.logins));
    let failed = Arc::new(AtomicUsize::new(0));
    let mut joins = Vec::with_capacity(n);
    for (i, (map, at)) in points.iter().copied().enumerate() {
        let realm = realm.clone();
        let gate = gate.clone();
        let failed = failed.clone();
        let gm = !args.no_gm;
        joins.push(tokio::spawn(async move {
            let _permit = gate.acquire_owned().await.ok()?;
            match place(&realm, i, map, at, gm).await {
                Ok(p) => Some(p),
                Err(e) => {
                    if failed.fetch_add(1, Ordering::Relaxed) < 10 {
                        eprintln!("observer {i} at {map} {at:?}: {e}");
                    }
                    None
                }
            }
        }));
    }
    let mut placed = Vec::with_capacity(n);
    let mut queues = Vec::with_capacity(n);
    for j in joins {
        if let Ok(Some((p, rx))) = j.await {
            placed.push(p);
            queues.push(rx);
        }
    }
    println!(
        "{} observers in place in {:.1} s ({} failed)",
        placed.len(),
        t.elapsed().as_secs_f64(),
        failed.load(Ordering::Relaxed)
    );

    for m in [0, 1] {
        let h = realm.maps.get(&realm, MapKey::world(m));
        if let Some((players, known, waiting)) = h
            .query(|map| {
                let known: usize = map.players.values().map(|p| p.known.len()).sum();
                let waiting = map
                    .players
                    .values()
                    .filter(|p| p.awaiting_worldport || p.teleport.is_some())
                    .count();
                (map.players.len(), known, waiting)
            })
            .await
        {
            println!(
                "map {m}: {players} players, knowing {:.0} objects each, {waiting} still to answer a teleport",
                known as f64 / players.max(1) as f64
            );
        }
    }

    // The observers' queues, drained by one thread every 50 ms.
    let draining = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let drainer = {
        let draining = draining.clone();
        std::thread::spawn(move || {
            while draining.load(Ordering::Relaxed) {
                let (mut bytes, mut frames) = (0, 0);
                for q in &queues {
                    while let Ok(Some(f)) = q.try_recv() {
                        bytes += f.len() as u64;
                        frames += 1;
                    }
                }
                BYTES_IN.fetch_add(bytes, Ordering::Relaxed);
                FRAMES_IN.fetch_add(frames, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(50));
            }
        })
    };
    // The observers stay as long as their senders do.
    let (walker, _placed) = if args.walk {
        (Some(tokio::spawn(walk(placed))), None)
    } else {
        (None, Some(placed))
    };

    tokio::time::sleep(Duration::from_secs(args.settle)).await;
    let load_before = loadavg();
    bench_probe::take();
    BYTES_IN.store(0, Ordering::Relaxed);
    FRAMES_IN.store(0, Ordering::Relaxed);
    let cpu0 = process_cpu();
    let io0 = process_io();
    let t0 = Instant::now();
    tokio::time::sleep(Duration::from_secs(args.secs)).await;
    let (samples, events) = bench_probe::take();
    bench_probe::disable();
    let elapsed = t0.elapsed().as_secs_f64();
    let cores_used = (process_cpu() - cpu0) / elapsed;
    let io1 = process_io();
    let io = ((io1.0 - io0.0) / elapsed, (io1.1 - io0.1) / elapsed / 1e6);
    let bytes = BYTES_IN.load(Ordering::Relaxed);
    let frames = FRAMES_IN.load(Ordering::Relaxed);
    let load_after = loadavg();
    let rss = peak_rss_mb();
    // The tiles each continent holds at the end: navmesh in and being
    // read, terrain read.
    let mut tiles = Vec::new();
    for m in [0, 1] {
        let h = realm.maps.get(&realm, MapKey::world(m));
        if let Some(t) = h
            .query(|map| {
                (
                    map.nav.as_ref().map_or(0, |n| n.tiles_in()),
                    map.nav.as_ref().map_or(0, |n| n.tiles_loading()),
                    map.terrain.as_ref().map_or(0, |t| t.tiles_read()),
                )
            })
            .await
        {
            tiles.push((m, t));
        }
    }

    let report = Report::new(
        &args,
        n,
        &samples,
        &events,
        elapsed,
        cores_used,
        (rss, rss_mb()),
        &tiles,
        (bytes, frames),
        io,
        (load_before.trim(), load_after.trim()),
    );
    report.print();
    if let Some(path) = &args.json
        && let Err(e) = std::fs::write(path, report.json.to_string() + "\n")
    {
        eprintln!("{}: {e}", path.display());
    }

    if let Some(w) = walker {
        w.abort();
    }
    draining.store(false, Ordering::Relaxed);
    let _ = drainer.join();
    std::process::exit(0);
}

/// Wait for a packet `pick` takes, reading `rx`.
async fn wait_for<T>(
    rx: &kanal::AsyncReceiver<Arc<[u8]>>,
    what: &str,
    mut pick: impl FnMut(&ServerOpcodeMessage) -> Option<Result<T, String>>,
) -> Result<T, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let frame = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .map_err(|_| format!("no {what} within 60 s"))?
            .map_err(|_| format!("the session closed waiting for {what}"))?;
        if let Ok(m) = ServerOpcodeMessage::read_unencrypted(&frame[..])
            && let Some(r) = pick(&m)
        {
            return r;
        }
    }
}

/// Log observer `i` in as a new character, in GM mode if `gm`, and send it
/// to `at` on `map`: its handle, and its queue to drain.
async fn place(
    realm: &Arc<Realm>,
    i: usize,
    map: u32,
    at: Vector3d,
    gm: bool,
) -> Result<(Placed, kanal::Receiver<Arc<[u8]>>), String> {
    let account = format!("WB{i:05}");
    let (outbound, rx) = Outbound::detached();
    let rx = rx.to_async();
    let (tx, inbound) = mpsc::channel(256);
    tokio::spawn(session::run(
        realm.clone(),
        account,
        Connection { outbound, inbound },
    ));
    let send = |m: ClientOpcodeMessage| {
        let tx = tx.clone();
        async move {
            tx.send(Inbound::Msg(m))
                .await
                .map_err(|_| "the session has gone".to_owned())
        }
    };
    let name = name_of(i);
    send(
        CMSG_CHAR_CREATE {
            name: name.clone(),
            race: Race::Human,
            class: Class::Warrior,
            gender: Gender::Female,
            skin_color: 1,
            face: 1,
            hair_style: 1,
            hair_color: 1,
            facial_hair: 1,
        }
        .into(),
    )
    .await?;
    wait_for(&rx, "SMSG_CHAR_CREATE", |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_CREATE(c) => Some(
            (c.result == WorldResult::CharCreateSuccess)
                .then_some(())
                .ok_or_else(|| format!("creation: {:?}", c.result)),
        ),
        _ => None,
    })
    .await?;
    send(CMSG_CHAR_ENUM {}.into()).await?;
    let guid = wait_for(&rx, "SMSG_CHAR_ENUM", |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => Some(
            e.characters
                .first()
                .map(|c| c.guid)
                .ok_or_else(|| "no character".to_owned()),
        ),
        _ => None,
    })
    .await?;
    send(CMSG_PLAYER_LOGIN { guid }.into()).await?;
    wait_for(&rx, "SMSG_LOGIN_VERIFY_WORLD", |m| match m {
        ServerOpcodeMessage::SMSG_LOGIN_VERIFY_WORLD(_) => Some(Ok(())),
        ServerOpcodeMessage::SMSG_CHARACTER_LOGIN_FAILED(f) => {
            Some(Err(format!("login: {:?}", f.result)))
        }
        _ => None,
    })
    .await?;
    send(CMSG_SET_ACTIVE_MOVER { guid }.into()).await?;
    // A new character's intro, which a client ends: until then it sees
    // from the intro's camera.
    send(CMSG_COMPLETE_CINEMATIC {}.into()).await?;
    let say = |message: String| CMSG_MESSAGECHAT {
        chat_type: CMSG_MESSAGECHAT_ChatType::Say,
        language: Language::Universal,
        message,
    };
    if gm {
        send(say(".gm on".into()).into()).await?;
    }
    send(say(format!(".go xyz {} {} {} {map}", at.x, at.y, at.z)).into()).await?;
    let new_world = wait_for(&rx, "the teleport", |m| match m {
        ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) if t.guid == guid => Some(Ok(false)),
        ServerOpcodeMessage::SMSG_NEW_WORLD(_) => Some(Ok(true)),
        ServerOpcodeMessage::SMSG_TRANSFER_ABORTED(a) => {
            Some(Err(format!("transfer refused: {:?}", a.reason)))
        }
        _ => None,
    })
    .await?;
    if new_world {
        send(MSG_MOVE_WORLDPORT_ACK {}.into()).await?;
    } else {
        send(
            MSG_MOVE_TELEPORT_ACK_Client {
                guid,
                movement_counter: 0,
                time: Default::default(),
            }
            .into(),
        )
        .await?;
    }
    let heading = (i as f32 * 2.399_963).rem_euclid(std::f32::consts::TAU);
    Ok((Placed { tx, at, heading }, rx.to_sync()))
}

/// The observers walk at running speed (7 yd/s), a heartbeat every 250 ms,
/// each turning now and then (every 3 s or so, by up to 90 degrees). The
/// height stays the point's: the server takes a client's.
async fn walk(mut placed: Vec<Placed>) {
    use rand::{RngExt, SeedableRng};
    const TICK: Duration = Duration::from_millis(250);
    const STEP: f32 = 7.0 * 0.25;
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        for p in &mut placed {
            if rng.random_bool(0.08) {
                p.heading = (p.heading
                    + rng.random_range(-std::f32::consts::FRAC_PI_2..std::f32::consts::FRAC_PI_2))
                .rem_euclid(std::f32::consts::TAU);
            }
            p.at.x += STEP * p.heading.cos();
            p.at.y += STEP * p.heading.sin();
            let _ = p.tx.try_send(Inbound::Msg(
                MSG_MOVE_HEARTBEAT_Client {
                    info: MovementInfo {
                        flags: MovementInfo_MovementFlags::new_forward(),
                        timestamp: 0,
                        position: p.at,
                        orientation: p.heading,
                        fall_time: 0.0,
                    },
                }
                .into(),
            ));
        }
    }
}

/// The process's CPU time (user and system), in seconds.
fn process_cpu() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // After the command's closing parenthesis: utime and stime are the
    // 12th and 13th fields from there.
    let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    // SAFETY: sysconf has no preconditions.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    (ticks(11) + ticks(12)) / hz
}

/// The process's major page faults (each a wait for the disk), and the
/// bytes it has had read from the disk (`/proc/self/io`'s `read_bytes`).
fn process_io() -> (f64, f64) {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
    let majflt = rest
        .split_whitespace()
        .nth(9)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    let read = std::fs::read_to_string("/proc/self/io")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("read_bytes:"))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(0.0);
    (majflt, read)
}

/// The process's peak resident memory, in MB.
fn peak_rss_mb() -> f64 {
    status_mb("VmHWM:")
}

/// The process's resident memory now, in MB.
fn rss_mb() -> f64 {
    status_mb("VmRSS:")
}

fn status_mb(key: &str) -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

struct Report {
    text: String,
    json: serde_json::Value,
}

impl Report {
    #[allow(clippy::too_many_arguments)]
    fn new(
        args: &Args,
        n: usize,
        samples: &[Sample],
        events: &AHashMap<MapKey, (u64, Duration)>,
        elapsed: f64,
        cores: f64,
        (rss_mb, rss_now_mb): (f64, f64),
        tiles: &[(u32, (usize, usize, usize))],
        (bytes, frames): (u64, u64),
        (faults, read_mb): (f64, f64),
        (load_before, load_after): (&str, &str),
    ) -> Self {
        use std::fmt::Write;
        let mut text = String::new();
        let mut maps = Vec::new();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let _ = writeln!(
            text,
            "\n{n} observers{}, {elapsed:.0} s; load average {load_before} at the start, {load_after} at the end",
            if args.walk { ", walking" } else { "" }
        );
        let _ = writeln!(
            text,
            "{:<10} {:>8} {:>7} | {:>27} | {:>27} | {:>6} | {:>13} | {:>17} | {:>5}",
            "map",
            "players",
            "updates",
            "wall ms p50/p95/p99/max",
            "CPU ms p50/p95/p99/max",
            ">50ms",
            "awake mean/max",
            "events/s, CPU ms/s",
            "busy"
        );
        for key in [MapKey::world(0), MapKey::world(1)] {
            let mine: Vec<&Sample> = samples.iter().filter(|s| s.key == key).collect();
            if mine.is_empty() {
                continue;
            }
            let mut wall: Vec<f64> = mine.iter().map(|s| ms(s.wall)).collect();
            let mut cpu: Vec<f64> = mine.iter().map(|s| ms(s.cpu)).collect();
            wall.sort_by(f64::total_cmp);
            cpu.sort_by(f64::total_cmp);
            let over = wall.iter().filter(|w| **w > 50.0).count() as f64 / wall.len() as f64;
            let cpu_over = cpu.iter().filter(|c| **c > 50.0).count() as f64 / cpu.len() as f64;
            let awake_mean =
                mine.iter().map(|s| f64::from(s.awake)).sum::<f64>() / mine.len() as f64;
            let awake_max = mine.iter().map(|s| s.awake).max().unwrap_or(0);
            let players = mine.last().map_or(0, |s| s.players);
            let (ev_n, ev_cpu) = events.get(&key).copied().unwrap_or_default();
            let update_cpu: f64 = cpu.iter().sum::<f64>() / 1000.0;
            let busy = (update_cpu + ev_cpu.as_secs_f64()) / elapsed;
            let q = |v: &[f64]| {
                format!(
                    "{:.1}/{:.1}/{:.1}/{:.1}",
                    pct(v, 0.5),
                    pct(v, 0.95),
                    pct(v, 0.99),
                    v.last().copied().unwrap_or(0.0)
                )
            };
            let _ = writeln!(
                text,
                "{:<10} {:>8} {:>7} | {:>27} | {:>27} | {:>5.1}% | {:>6.0}/{:<6} | {:>7.0}, {:>8.1} | {:>4.0}%",
                key.map,
                players,
                mine.len(),
                q(&wall),
                q(&cpu),
                over * 100.0,
                awake_mean,
                awake_max,
                ev_n as f64 / elapsed,
                ms(ev_cpu) / elapsed,
                busy * 100.0
            );
            // The steps, by mean CPU an update.
            let mut steps: AHashMap<&'static str, f64> = AHashMap::new();
            for s in &mine {
                for (name, d) in &s.steps {
                    *steps.entry(name).or_default() += ms(*d);
                }
            }
            let mut steps: Vec<(&str, f64)> = steps
                .into_iter()
                .map(|(k, v)| (k, v / mine.len() as f64))
                .collect();
            steps.sort_by(|a, b| b.1.total_cmp(&a.1));
            let total: f64 = steps.iter().map(|s| s.1).sum();
            let top: Vec<String> = steps
                .iter()
                .take(8)
                .map(|(k, v)| format!("{k} {v:.2} ({:.0}%)", v / total.max(1e-9) * 100.0))
                .collect();
            let _ = writeln!(text, "    steps, CPU ms an update: {}", top.join(", "));
            // Where an update's wall time off the CPU went: waits (the disk,
            // a lock) or preemption (the machine busy).
            let waits = mine.iter().map(|s| s.switches.0).sum::<u64>() as f64 / mine.len() as f64;
            let preempted =
                mine.iter().map(|s| s.switches.1).sum::<u64>() as f64 / mine.len() as f64;
            let wall_total: f64 = wall.iter().sum();
            let off_cpu = (wall_total - cpu.iter().sum::<f64>()).max(0.0) / wall_total.max(1e-9);
            let slow: Vec<&&Sample> = mine.iter().filter(|s| ms(s.wall) > 50.0).collect();
            let slow_off = if slow.is_empty() {
                0.0
            } else {
                slow.iter()
                    .map(|s| (ms(s.wall) - ms(s.cpu)).max(0.0))
                    .sum::<f64>()
                    / slow.iter().map(|s| ms(s.wall)).sum::<f64>()
            };
            let _ = writeln!(
                text,
                "    off the CPU: {:.0}% of the update's wall time ({:.0}% of the updates over 50 ms'); a mean {waits:.2} waits and {preempted:.2} preemptions an update",
                off_cpu * 100.0,
                slow_off * 100.0
            );
            maps.push(serde_json::json!({
                "map": key.map,
                "players": players,
                "updates": mine.len(),
                "updates_per_sec": mine.len() as f64 / elapsed,
                "wall_ms": {"p50": pct(&wall, 0.5), "p95": pct(&wall, 0.95), "p99": pct(&wall, 0.99), "max": wall.last()},
                "cpu_ms": {"p50": pct(&cpu, 0.5), "p95": pct(&cpu, 0.95), "p99": pct(&cpu, 0.99), "max": cpu.last()},
                "over_50ms": over,
                "cpu_over_50ms": cpu_over,
                "awake_mean": awake_mean,
                "awake_max": awake_max,
                "events_per_sec": ev_n as f64 / elapsed,
                "events_cpu_ms_per_sec": ms(ev_cpu) / elapsed,
                "busy": busy,
                "off_cpu": off_cpu,
                "slow_off_cpu": slow_off,
                "waits_per_update": waits,
                "preemptions_per_update": preempted,
                "steps_ms": steps.iter().map(|(k, v)| (k.to_string(), *v)).collect::<std::collections::BTreeMap<_, _>>(),
                // Each update in turn: its CPU ms, and its three dearest
                // steps' (for what comes and goes with time).
                "series": mine.iter().map(|s| {
                    let mut top = s.steps.clone();
                    top.sort_by_key(|s| std::cmp::Reverse(s.1));
                    top.truncate(3);
                    serde_json::json!([
                        (ms(s.cpu) * 100.0).round() / 100.0,
                        top.iter().map(|(k, d)| (k.to_string(), (ms(*d) * 100.0).round() / 100.0)).collect::<Vec<_>>(),
                    ])
                }).collect::<Vec<_>>(),
            }));
        }
        for (m, (nav_in, nav_loading, terrain)) in tiles {
            let _ = writeln!(
                text,
                "map {m}'s tiles: navmesh {nav_in} in, {nav_loading} being read; terrain {terrain} read"
            );
        }
        let _ = writeln!(
            text,
            "process: {cores:.2} cores, peak RSS {rss_mb:.0} MB (at the end {rss_now_mb:.0}); to the observers {:.0} kB/s in {:.0} frames/s",
            bytes as f64 / elapsed / 1e3,
            frames as f64 / elapsed
        );
        let _ = writeln!(
            text,
            "the disk: {faults:.0} major page faults a second, {read_mb:.1} MB/s read"
        );
        let json = serde_json::json!({
            "observers": n,
            "walk": args.walk,
            "gm": !args.no_gm,
            "secs": elapsed,
            "cores": cores,
            "peak_rss_mb": rss_mb,
            "rss_end_mb": rss_now_mb,
            "tiles": tiles.iter().map(|(m, (a, b, c))| serde_json::json!({"map": m, "nav_in": a, "nav_loading": b, "terrain_read": c})).collect::<Vec<_>>(),
            "bytes_per_sec": bytes as f64 / elapsed,
            "frames_per_sec": frames as f64 / elapsed,
            "major_faults_per_sec": faults,
            "disk_read_mb_per_sec": read_mb,
            "load_before": load_before,
            "load_after": load_after,
            "maps": maps,
        });
        Self { text, json }
    }

    fn print(&self) {
        print!("{}", self.text);
    }
}
