//! A crowd in one spot, through the real sessions and map (over channels,
//! as the world benchmark has them: no sockets, no cipher), measuring what
//! the movement drill (docs/PERFORMANCE.md, "The movement drill") measured
//! of arrival and of the mailbox, at crowds of 100 to 3,000.
//!
//! ```sh
//! WVS_DATA=data cargo bench --features world-bench --bench crowd             # both, at 100, 880, 2,000, 3,000
//! CROWD=880,2000 CROWD_MODE=arrival cargo bench --features world-bench --bench crowd
//! ```
//!
//! - `arrival`: N players stand on a 60-yard disc, out of GM mode; then
//!   `CROWD_NEWCOMERS` (200) more come in at once as the drill's bots do:
//!   created, logged in at the disc, `.character level 30`, `.gear`,
//!   `.god on`, `.gm off`, `.go xyz` to a spot on it, the teleport
//!   acknowledged. It reports the newcomers placed a second, each step's
//!   wait, and the map thread's CPU by event kind and by update step.
//! - `movement`: the N run about the disc, a heartbeat each every
//!   `CROWD_BEAT_MS` (250) ms, for `CROWD_SECS` (10) s: the map's events
//!   handled a second, its mailbox after each update, its updates' time,
//!   and the broadcasts the crowd got.
//!
//! With `WVS_DATA` (a `data/` with `db`), the real world's items, spells
//! and creatures; without, an empty world (`.gear` gives nothing).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use wow_vanilla_server::game::conn::{Connection, Frames, Inbound, Outbound};
use wow_vanilla_server::game::data::GameData;
use wow_vanilla_server::game::map::MapKey;
use wow_vanilla_server::game::map::bench_probe;
use wow_vanilla_server::game::store::Store;
use wow_vanilla_server::game::{Realm, RealmConfig, session};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::*;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The disc: open sand in Tanaris, the drill's.
const MAP: u32 = 1;
const CENTER: (f32, f32, f32) = (-7920.0, -4460.0, 9.14);
const RADIUS: f32 = 60.0;

/// Frames the crowd's queues have given up, and their bytes.
static FRAMES: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
/// Movement broadcasts the crowd got.
static MOVES: AtomicU64 = AtomicU64::new(0);

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    // `cargo bench` passes `--bench`; there is nothing to filter.
    let sizes: Vec<usize> = std::env::var("CROWD")
        .unwrap_or_else(|_| "100,880,2000,3000".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let mode = std::env::var("CROWD_MODE").unwrap_or_else(|_| "both".into());
    {
        use tracing_subscriber::Layer;
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_filter(wow_vanilla_server::server::log_filter("error")),
            )
            .init();
    }
    let mut cfg = wow_vanilla_server::config::ServerConfig::default();
    cfg.spawn.enabled = true;
    cfg.spawn.map = MAP;
    cfg.spawn.x = CENTER.0;
    cfg.spawn.y = CENTER.1;
    cfg.spawn.z = CENTER.2;
    cfg.world.save_interval_secs = 3600;
    let data_dir = std::env::var_os("WVS_DATA").map(|d| std::path::PathBuf::from(d).join("db"));
    cfg.world.data_dir = data_dir.clone();
    let cfg = wow_vanilla_server::config::install(cfg);
    println!(
        "world: {}",
        if data_dir.is_some() {
            "the real world"
        } else {
            "empty (no WVS_DATA)"
        }
    );
    let data = move || match &data_dir {
        Some(d) => wow_vanilla_server::game::data::load::load(d),
        None => GameData::empty(),
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    for n in sizes {
        if mode == "both" || mode == "arrival" {
            rt.block_on(arrival(cfg, data(), n));
        }
        if mode == "both" || mode == "movement" {
            rt.block_on(movement(cfg, data(), n));
        }
    }
    std::process::exit(0);
}

fn realm(cfg: &'static wow_vanilla_server::config::ServerConfig, data: GameData) -> Arc<Realm> {
    let (store, startup) = Store::open(":memory:").expect("an in-memory store");
    let mut config = RealmConfig::from_config(cfg);
    config.everyone_gm = true;
    Realm::new(config, store, startup, data, None)
}

/// A client: what it sends, and its queue.
struct Bot {
    tx: mpsc::Sender<Inbound>,
    rx: Frames,
    guid: Guid,
    at: Vector3d,
    heading: f32,
}

fn spot(i: usize) -> Vector3d {
    // A sunflower over the disc: even, and the same each run.
    let r = RADIUS * ((i as f32 + 0.5) / 4000.0).sqrt();
    let a = i as f32 * 2.399_963;
    Vector3d {
        x: CENTER.0 + r * a.cos(),
        y: CENTER.1 + r * a.sin(),
        z: CENTER.2,
    }
}

fn opcode(frame: &[u8]) -> u16 {
    u16::from_le_bytes([frame[2], frame[3]])
}

const SMSG_MESSAGECHAT: u16 = 0x096;
const SMSG_CHAR_CREATE: u16 = 0x03A;
const SMSG_CHAR_ENUM: u16 = 0x03B;
const SMSG_LOGIN_VERIFY_WORLD: u16 = 0x236;
const MSG_MOVE_TELEPORT_ACK: u16 = 0x0C7;

/// Wait for a frame of `op` that `pick` takes; others are dropped unread.
async fn wait_for<T>(
    rx: &mut Frames,
    op: u16,
    mut pick: impl FnMut(&ServerOpcodeMessage) -> Option<T>,
) -> Result<T, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let frame = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .map_err(|_| format!("no {op:#x} within 120 s"))?
            .ok_or_else(|| "the session closed".to_owned())?;
        if frame.len() < 4 || opcode(&frame) != op {
            continue;
        }
        if let Ok(m) = ServerOpcodeMessage::read_unencrypted(&frame[..])
            && let Some(r) = pick(&m)
        {
            return Ok(r);
        }
    }
}

async fn send(tx: &mpsc::Sender<Inbound>, m: impl Into<ClientOpcodeMessage>) -> Result<(), String> {
    tx.send(Inbound::Msg(m.into()))
        .await
        .map_err(|_| "the session has gone".to_owned())
}

async fn say(bot: &mut Bot, text: &str, reply: &str) -> Result<(), String> {
    send(
        &bot.tx,
        CMSG_MESSAGECHAT {
            chat_type: CMSG_MESSAGECHAT_ChatType::Say,
            language: Language::Universal,
            message: text.to_owned(),
        },
    )
    .await?;
    wait_for(&mut bot.rx, SMSG_MESSAGECHAT, |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
            if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::System { .. })
                && c.message.contains(reply) =>
        {
            Some(())
        }
        _ => None,
    })
    .await
    .map_err(|e| format!("`{text}`: {e}"))
}

/// Create a character on a fresh account and log it in, at the spawn.
async fn enter(realm: &Arc<Realm>, prefix: &str, i: usize) -> Result<Bot, String> {
    let (outbound, rx) = Outbound::detached();
    let (tx, inbound) = mpsc::channel(256);
    tokio::spawn(session::run(
        realm.clone(),
        format!("{prefix}{i:05}"),
        Connection { outbound, inbound },
    ));
    let mut bot = Bot {
        tx,
        rx,
        guid: Guid::zero(),
        at: spot(i),
        heading: (i as f32 * 1.7).rem_euclid(std::f32::consts::TAU),
    };
    let letters: String = format!("{i:05}")
        .bytes()
        .map(|b| char::from(b'a' + (b - b'0')))
        .collect();
    let name = format!("{prefix}{letters}");
    let name = {
        let mut c = name.to_ascii_lowercase().into_bytes();
        c[0] = c[0].to_ascii_uppercase();
        String::from_utf8(c).expect("ascii")
    };
    send(
        &bot.tx,
        CMSG_CHAR_CREATE {
            name,
            race: Race::Human,
            class: Class::Warrior,
            gender: Gender::Female,
            skin_color: 1,
            face: 1,
            hair_style: 1,
            hair_color: 1,
            facial_hair: 1,
        },
    )
    .await?;
    let made = wait_for(&mut bot.rx, SMSG_CHAR_CREATE, |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_CREATE(c) => Some(c.result),
        _ => None,
    })
    .await?;
    if made != WorldResult::CharCreateSuccess {
        return Err(format!("creation: {made:?}"));
    }
    send(&bot.tx, CMSG_CHAR_ENUM {}).await?;
    let guid = wait_for(&mut bot.rx, SMSG_CHAR_ENUM, |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => e.characters.first().map(|c| c.guid),
        _ => None,
    })
    .await?;
    bot.guid = guid;
    send(&bot.tx, CMSG_PLAYER_LOGIN { guid }).await?;
    wait_for(&mut bot.rx, SMSG_LOGIN_VERIFY_WORLD, |_| Some(())).await?;
    send(&bot.tx, CMSG_SET_ACTIVE_MOVER { guid }).await?;
    send(&bot.tx, CMSG_COMPLETE_CINEMATIC {}).await?;
    Ok(bot)
}

/// `.go xyz` to the bot's spot, and the teleport acknowledged.
async fn go(bot: &mut Bot) -> Result<(), String> {
    let at = bot.at;
    send(
        &bot.tx,
        CMSG_MESSAGECHAT {
            chat_type: CMSG_MESSAGECHAT_ChatType::Say,
            language: Language::Universal,
            message: format!(".go xyz {} {} {} {MAP}", at.x, at.y, at.z),
        },
    )
    .await?;
    let guid = bot.guid;
    let counter = wait_for(&mut bot.rx, MSG_MOVE_TELEPORT_ACK, |m| match m {
        ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) if t.guid == guid => Some(t.movement_counter),
        _ => None,
    })
    .await?;
    send(
        &bot.tx,
        MSG_MOVE_TELEPORT_ACK_Client {
            guid,
            movement_counter: counter,
            time: Default::default(),
        },
    )
    .await
}

/// `n` players on the disc, out of GM mode, 64 logging in at once.
async fn crowd(realm: &Arc<Realm>, n: usize) -> Vec<Bot> {
    let gate = Arc::new(tokio::sync::Semaphore::new(64));
    let mut joins = Vec::with_capacity(n);
    for i in 0..n {
        let realm = realm.clone();
        let gate = gate.clone();
        joins.push(tokio::spawn(async move {
            let _p = gate.acquire_owned().await.ok()?;
            let mut bot = enter(&realm, "Cr", i).await.ok()?;
            say(&mut bot, ".gm off", "GM mode off").await.ok()?;
            go(&mut bot).await.ok()?;
            Some(bot)
        }));
    }
    let mut bots = Vec::with_capacity(n);
    for j in joins {
        if let Ok(Some(b)) = j.await {
            bots.push(b);
        }
    }
    bots
}

/// The crowd's queues, drained every 20 ms on a thread of their own; the
/// movement broadcasts counted. The senders go back when it stops.
fn drain(queues: Vec<Frames>, on: Arc<AtomicBool>) -> std::thread::JoinHandle<Vec<Frames>> {
    std::thread::spawn(move || {
        let mut queues = queues;
        while on.load(Ordering::Relaxed) {
            let (mut bytes, mut frames, mut moves) = (0, 0, 0);
            for q in &mut queues {
                while let Some(f) = q.try_recv() {
                    bytes += f.len() as u64;
                    frames += 1;
                    if f.len() >= 4 && (0xB5..=0xEE).contains(&opcode(&f)) {
                        moves += 1;
                    }
                }
            }
            BYTES.fetch_add(bytes, Ordering::Relaxed);
            FRAMES.fetch_add(frames, Ordering::Relaxed);
            MOVES.fetch_add(moves, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(20));
        }
        queues
    })
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// The map's update and events, over what the probe took.
fn report_map(key: MapKey, secs: f64) {
    let (samples, _) = bench_probe::take();
    let kinds = bench_probe::take_kinds();
    let mailbox = bench_probe::take_mailbox();
    let mut walls: Vec<f64> = samples
        .iter()
        .filter(|s| s.key == key)
        .map(|s| s.wall.as_secs_f64() * 1e3)
        .collect();
    walls.sort_by(f64::total_cmp);
    let update_cpu: f64 = samples
        .iter()
        .filter(|s| s.key == key)
        .map(|s| s.cpu.as_secs_f64())
        .sum();
    let mut steps: ahash::AHashMap<&str, f64> = ahash::AHashMap::new();
    for s in samples.iter().filter(|s| s.key == key) {
        for (name, d) in &s.steps {
            *steps.entry(name).or_default() += d.as_secs_f64();
        }
    }
    let mut steps: Vec<_> = steps.into_iter().collect();
    steps.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut kinds: Vec<_> = kinds.into_iter().filter(|((k, _), _)| *k == key).collect();
    kinds.sort_by_key(|a| std::cmp::Reverse(a.1.1));
    let events: u64 = kinds.iter().map(|(_, (n, _))| n).sum();
    let event_cpu: f64 = kinds.iter().map(|(_, (_, d))| d.as_secs_f64()).sum();
    let mut boxes: Vec<f64> = mailbox
        .iter()
        .filter(|(k, _)| *k == key)
        .map(|(_, n)| *n as f64)
        .collect();
    boxes.sort_by(f64::total_cmp);
    println!(
        "  updates {} ({:.1}/s): p50 {:.2} / p99 {:.2} / max {:.2} ms; CPU {:.2} s in updates, {:.2} s in {} events ({:.0}/s); thread busy {:.0}%",
        walls.len(),
        walls.len() as f64 / secs,
        pct(&walls, 0.5),
        pct(&walls, 0.99),
        walls.last().copied().unwrap_or(0.0),
        update_cpu,
        event_cpu,
        events,
        events as f64 / secs,
        100.0 * (update_cpu + event_cpu) / secs
    );
    println!(
        "  mailbox after an update: p50 {:.0} / p99 {:.0} / max {:.0}",
        pct(&boxes, 0.5),
        pct(&boxes, 0.99),
        boxes.last().copied().unwrap_or(0.0)
    );
    let top: Vec<String> = steps
        .iter()
        .take(6)
        .map(|(n, s)| format!("{n} {:.0} ms", s * 1e3))
        .collect();
    println!("  dearest steps (CPU): {}", top.join(", "));
    let top: Vec<String> = kinds
        .iter()
        .take(8)
        .map(|((_, k), (n, d))| {
            format!(
                "{k} {n} x {:.0} µs = {:.0} ms",
                d.as_secs_f64() * 1e6 / (*n).max(1) as f64,
                d.as_secs_f64() * 1e3
            )
        })
        .collect();
    println!("  dearest events (CPU): {}", top.join("; "));
}

async fn arrival(cfg: &'static wow_vanilla_server::config::ServerConfig, data: GameData, n: usize) {
    let newcomers: usize = env("CROWD_NEWCOMERS", 200);
    let realm = realm(cfg, data);
    let key = MapKey::world(MAP);
    realm.maps.get(&realm, key).query(|_| ()).await;
    let t = Instant::now();
    let bots = crowd(&realm, n).await;
    let setup = t.elapsed().as_secs_f64();
    let on = Arc::new(AtomicBool::new(true));
    let drainer = drain(bots.into_iter().map(|b| b.rx).collect(), on.clone());
    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("arrival: a crowd of {n} (in place in {setup:.1} s); {newcomers} newcomers at once");
    bench_probe::enable();
    bench_probe::take();
    bench_probe::take_kinds();
    bench_probe::take_mailbox();
    let t0 = Instant::now();
    let mut joins = Vec::with_capacity(newcomers);
    for i in 0..newcomers {
        let realm = realm.clone();
        joins.push(tokio::spawn(async move {
            let mut marks = Vec::with_capacity(7);
            let start = Instant::now();
            let mut bot = enter(&realm, "Nw", n + i).await?;
            marks.push(start.elapsed());
            for (c, r) in [
                (".character level 30", "Level"),
                (".gear", "No gear"),
                (".god on", "Undying on"),
                (".gm off", "GM mode off"),
            ] {
                let r = if c == ".gear" && realm.data.items.is_empty() {
                    r
                } else if c == ".gear" {
                    "Equipped"
                } else {
                    r
                };
                say(&mut bot, c, r).await?;
                marks.push(start.elapsed());
            }
            go(&mut bot).await?;
            marks.push(start.elapsed());
            Ok::<_, String>((marks, bot))
        }));
    }
    let mut marks = Vec::new();
    let mut placed = Vec::new();
    let mut failed = 0;
    for j in joins {
        match j.await {
            Ok(Ok((m, b))) => {
                marks.push(m);
                placed.push(b);
            }
            Ok(Err(e)) => {
                if failed < 3 {
                    eprintln!("  a newcomer: {e}");
                }
                failed += 1;
            }
            Err(_) => failed += 1,
        }
    }
    let took = t0.elapsed().as_secs_f64();
    // One more update, for the last placement's creates.
    tokio::time::sleep(Duration::from_millis(100)).await;
    println!(
        "  {} placed in {took:.2} s: {:.0} a second ({failed} failed)",
        placed.len(),
        placed.len() as f64 / took
    );
    let names = [
        "logged in",
        ".character level",
        ".gear",
        ".god on",
        ".gm off",
        ".go, placed",
    ];
    let mut line = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let mut d: Vec<f64> = marks
            .iter()
            .filter_map(|m| {
                let at = m.get(i)?.as_secs_f64();
                let before = if i == 0 {
                    0.0
                } else {
                    m.get(i - 1)?.as_secs_f64()
                };
                Some((at - before) * 1e3)
            })
            .collect();
        d.sort_by(f64::total_cmp);
        line.push(format!("{name} {:.0}/{:.0}", pct(&d, 0.5), pct(&d, 0.99)));
    }
    println!("  each step's wait, p50/p99 ms: {}", line.join(", "));
    report_map(key, took);
    bench_probe::disable();
    on.store(false, Ordering::Relaxed);
    let _ = drainer.join();
    drop(placed);
    realm.shutdown_within(Duration::from_secs(10)).await;
    drop(realm);
}

async fn movement(
    cfg: &'static wow_vanilla_server::config::ServerConfig,
    data: GameData,
    n: usize,
) {
    let beat: u64 = env("CROWD_BEAT_MS", 250);
    let secs: u64 = env("CROWD_SECS", 10);
    let realm = realm(cfg, data);
    let key = MapKey::world(MAP);
    realm.maps.get(&realm, key).query(|_| ()).await;
    let mut bots = crowd(&realm, n).await;
    let mut senders = Vec::with_capacity(bots.len());
    let mut queues = Vec::with_capacity(bots.len());
    for b in bots.drain(..) {
        queues.push(b.rx);
        senders.push((b.tx, b.at, b.heading));
    }
    let on = Arc::new(AtomicBool::new(true));
    let drainer = drain(queues, on.clone());
    tokio::time::sleep(Duration::from_secs(2)).await;
    println!(
        "movement: a crowd of {} running about the disc, a heartbeat each every {beat} ms, {secs} s",
        senders.len()
    );
    bench_probe::enable();
    bench_probe::take();
    bench_probe::take_kinds();
    bench_probe::take_mailbox();
    MOVES.store(0, Ordering::Relaxed);
    let sent = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    // The crowd in 8 groups, each its own task, a tick each `beat`.
    let groups = 8;
    let mut tasks = Vec::new();
    let per = senders.len().div_ceil(groups);
    let mut senders = senders;
    while !senders.is_empty() {
        let group: Vec<_> = senders.drain(..per.min(senders.len())).collect();
        let sent = sent.clone();
        let stop = stop.clone();
        tasks.push(tokio::spawn(async move {
            let mut group = group;
            let mut tick = tokio::time::interval(Duration::from_millis(beat));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let step = 7.0 * beat as f32 / 1000.0;
            let started = Instant::now();
            while !stop.load(Ordering::Relaxed) {
                tick.tick().await;
                let ms = started.elapsed().as_millis() as u32;
                for (tx, at, heading) in &mut group {
                    let next = Vector3d {
                        x: at.x + step * heading.cos(),
                        y: at.y + step * heading.sin(),
                        z: at.z,
                    };
                    let dx = next.x - CENTER.0;
                    let dy = next.y - CENTER.1;
                    if dx * dx + dy * dy > RADIUS * RADIUS {
                        *heading =
                            (*heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU);
                    } else {
                        *at = next;
                    }
                    let m = MSG_MOVE_HEARTBEAT_Client {
                        info: MovementInfo {
                            flags: MovementInfo_MovementFlags::new_forward(),
                            timestamp: ms,
                            position: *at,
                            orientation: *heading,
                            fall_time: 0,
                        },
                    };
                    // As a socket's reader would: it waits when the session
                    // is full.
                    if tx.send(Inbound::Msg(m.into())).await.is_ok() {
                        sent.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            group
        }));
    }
    let t0 = Instant::now();
    tokio::time::sleep(Duration::from_secs(secs)).await;
    stop.store(true, Ordering::Relaxed);
    let took = t0.elapsed().as_secs_f64();
    let sent = sent.load(Ordering::Relaxed);
    println!(
        "  sent {sent} heartbeats ({:.0}/s); the crowd got {:.0} movement broadcasts a second",
        sent as f64 / took,
        MOVES.load(Ordering::Relaxed) as f64 / took
    );
    report_map(key, took);
    bench_probe::disable();
    let mut kept = Vec::new();
    for t in tasks {
        if let Ok(g) = t.await {
            kept.push(g);
        }
    }
    on.store(false, Ordering::Relaxed);
    let _ = drainer.join();
    drop(kept);
    realm.shutdown_within(Duration::from_secs(10)).await;
    drop(realm);
}
