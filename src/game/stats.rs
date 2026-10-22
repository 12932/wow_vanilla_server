//! A load test's numbers, a JSON line a second, to the file `WVS_STATS_FILE`
//! names (none without it): for each map, its players, the creatures awake,
//! the events waiting, every update's time, the time its events took, and
//! its biggest and total outbound queues; for the realm, the players online,
//! the packets in and out and the bytes out, the clients disconnected (by
//! why), the process's CPU time and its RSS. Tracy has the same and more,
//! but needs its tools and a profiler connected; this needs nothing, and
//! costs a relaxed load an update and an event while off.
//!
//! A line: `{"t":12.0,"unix":1792571234.5,"online":880,"packets_in":..,"packets_out":..,
//! "bytes_out":..,"heartbeats_coalesced":..,"packets_held_back":..,"disconnects":{"budget":0,"flood":0,"stall":0},
//! "cpu_secs":..,"rss_kb":..,"maps":[{"map":1,"instance":0,"players":880,
//! "awake":312,"mailbox":0,"updates_us":[..],"events":4400,"events_us":..,
//! "biggest_queue":..,"queued":..,"visibility_waiting":..,"steps_us":{"visibility":..},
//! "batch_ms":10.0,"batch_busy":0.42,"batch_cpu":0.31}]}`, its
//! counters since start-up (the reader takes differences), `updates_us` the second's;
//! `visibility_waiting` the most objects an update of the second left for the next to work out (over
//! `[visibility] refresh_budget_ms`, or marked after its visibility); `batch_ms` the map's broadcast
//! window as the second ends, `batch_busy` the smoothed share of its thread's time that steers it,
//! `batch_cpu` the server's smoothed CPU share over its cores, which steers it too.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::game::Realm;
use crate::game::map::{Map, MapKey};

/// The environment variable naming the file.
pub const VAR: &str = "WVS_STATS_FILE";

static ON: AtomicBool = AtomicBool::new(false);

/// Bytes written to all clients since start-up, their headers too.
pub static BYTES_OUT: AtomicU64 = AtomicU64::new(0);
/// Clients disconnected for going over their outbound byte budget, for
/// sending too fast (the flood guard's), and for taking nothing for a while.
pub static BUDGET: AtomicU64 = AtomicU64::new(0);
pub static FLOOD: AtomicU64 = AtomicU64::new(0);
pub static STALL: AtomicU64 = AtomicU64::new(0);
/// Heartbeats that took the place of their mover's last, still waiting
/// for its map; and packets a client's session waited to send, its share
/// of the map's mailbox full (`[network] max_queued_packets`).
pub static COALESCED: AtomicU64 = AtomicU64::new(0);
pub static HELD_BACK: AtomicU64 = AtomicU64::new(0);

/// The maps' seconds, waiting for the writer.
static SAMPLES: Mutex<Vec<Sample>> = Mutex::new(Vec::new());

thread_local! {
    /// The update steps' times on this thread, since its map's sampler
    /// last took them: an update runs on one thread, its sampler right
    /// after it.
    static STEPS: std::cell::RefCell<Vec<(&'static str, Duration)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// An update step, `name`, took `took`.
pub fn step_done(name: &'static str, took: Duration) {
    STEPS.with(|s| s.borrow_mut().push((name, took)));
}

/// Whether the stats are being written.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// A map's second.
#[derive(Debug, Default, Clone, serde::Serialize)]
struct Sample {
    map: u32,
    instance: u32,
    players: usize,
    awake: usize,
    mailbox: usize,
    updates_us: Vec<u32>,
    events: u64,
    events_us: u64,
    biggest_queue: usize,
    queued: usize,
    visibility_waiting: usize,
    /// Each update step's time over the second, by name.
    steps_us: std::collections::BTreeMap<&'static str, u64>,
    /// The map's broadcast window at the second's end, ms
    /// (`[network] broadcast_batch_*`), and the smoothed busy share that
    /// steers it.
    batch_ms: f64,
    batch_busy: f64,
    /// The server's smoothed CPU share, as the window last read it.
    batch_cpu: f64,
}

/// What a map's actor keeps of its second, until it hands it over.
#[derive(Debug)]
pub struct Sampler {
    updates_us: Vec<u32>,
    steps_us: std::collections::BTreeMap<&'static str, u64>,
    events: u64,
    events_us: u64,
    visibility_waiting: usize,
    due: Instant,
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            updates_us: Vec::with_capacity(24),
            steps_us: std::collections::BTreeMap::new(),
            events: 0,
            events_us: 0,
            visibility_waiting: 0,
            due: Instant::now() + Duration::from_secs(1),
        }
    }
}

fn micros(d: Duration) -> u32 {
    u32::try_from(d.as_micros()).unwrap_or(u32::MAX)
}

impl Sampler {
    /// `n` events, handled in a burst, took `took`.
    pub fn events(&mut self, n: u64, took: Duration) {
        self.events += n;
        self.events_us += took.as_micros() as u64;
    }

    /// After an update: its time, and once a second the map's second
    /// handed over, with `mailbox` events waiting.
    pub fn update(&mut self, map: &Map, mailbox: usize) {
        self.updates_us.push(micros(map.stats.last_update));
        self.visibility_waiting = self.visibility_waiting.max(map.visibility_dirty.len());
        STEPS.with(|steps| {
            for (name, took) in steps.borrow_mut().drain(..) {
                *self.steps_us.entry(name).or_default() += took.as_micros() as u64;
            }
        });
        if map.now < self.due {
            return;
        }
        self.due = map.now + Duration::from_secs(1);
        let (mut biggest, mut queued) = (0, 0);
        for p in map.players.values() {
            let q = p.outbound.queued_bytes();
            biggest = biggest.max(q);
            queued += q;
        }
        let MapKey { map: id, instance } = map.key;
        let sample = Sample {
            map: id,
            instance,
            players: map.players.len(),
            awake: map.awake.len(),
            mailbox,
            updates_us: std::mem::take(&mut self.updates_us),
            events: std::mem::take(&mut self.events),
            events_us: std::mem::take(&mut self.events_us),
            biggest_queue: biggest,
            queued,
            visibility_waiting: std::mem::take(&mut self.visibility_waiting),
            steps_us: std::mem::take(&mut self.steps_us),
            batch_ms: map.batch.window().as_secs_f64() * 1000.0,
            batch_busy: (map.batch.busy() * 1000.0).round() / 1000.0,
            batch_cpu: (map.batch.cpu() * 1000.0).round() / 1000.0,
        };
        SAMPLES.lock().push(sample);
    }
}

/// Start writing, if `WVS_STATS_FILE` names a file.
pub fn start(realm: &Arc<Realm>) {
    let Some(path) = std::env::var_os(VAR).filter(|p| !p.is_empty()) else {
        return;
    };
    let file = match std::fs::File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("{VAR}: can't create {}: {e}", path.to_string_lossy());
            return;
        }
    };
    tracing::info!("stats: a line a second to {}", path.to_string_lossy());
    ON.store(true, Ordering::Relaxed);
    tokio::spawn(write(realm.clone(), std::io::BufWriter::new(file)));
}

async fn write(realm: Arc<Realm>, mut out: std::io::BufWriter<std::fs::File>) {
    let started = Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let maps = std::mem::take(&mut *SAMPLES.lock());
        let line = serde_json::json!({
            "t": started.elapsed().as_secs_f64(),
            "unix": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64()),
            "online": realm.registry.online_count(),
            "packets_in": crate::game::conn::PACKETS_IN.load(Ordering::Relaxed),
            "packets_out": crate::game::conn::PACKETS_OUT.load(Ordering::Relaxed),
            "bytes_out": BYTES_OUT.load(Ordering::Relaxed),
            "heartbeats_coalesced": COALESCED.load(Ordering::Relaxed),
            "packets_held_back": HELD_BACK.load(Ordering::Relaxed),
            "disconnects": {
                "budget": BUDGET.load(Ordering::Relaxed),
                "flood": FLOOD.load(Ordering::Relaxed),
                "stall": STALL.load(Ordering::Relaxed),
            },
            "memory_guard": {
                "on": crate::game::memory_guard::on(),
                "engaged": crate::game::memory_guard::ENGAGED.load(Ordering::Relaxed),
            },
            "cpu_secs": cpu_secs(),
            "rss_kb": rss_kb(),
            "maps": maps,
        });
        let wrote = writeln!(out, "{line}").and_then(|()| out.flush());
        if let Err(e) = wrote {
            tracing::warn!("stats: can't write: {e}; stopping");
            ON.store(false, Ordering::Relaxed);
            return;
        }
    }
}

/// The process's user and system time, from `/proc/self/stat`.
#[cfg(unix)]
fn cpu_secs() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let rest = &stat[stat.rfind(')')? + 2..];
    let mut fields = rest.split_whitespace();
    let utime: f64 = fields.nth(11)?.parse().ok()?;
    let stime: f64 = fields.next()?.parse().ok()?;
    // SAFETY: sysconf reads a constant.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    Some((utime + stime) / if ticks > 0 { ticks as f64 } else { 100.0 })
}

/// No `/proc` (or `libc`) off Unix: the stats file's `cpu_secs` is null.
#[cfg(not(unix))]
fn cpu_secs() -> Option<f64> {
    None
}

/// The process's resident set, from `/proc/self/status`.
pub(crate) fn rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_process_reads() {
        if cfg!(target_os = "linux") {
            assert!(cpu_secs().is_some());
            assert!(rss_kb().is_some_and(|kb| kb > 0));
        }
    }

    #[test]
    fn a_sample_is_one_json_object() {
        let s = Sample {
            map: 1,
            updates_us: vec![1200, 900],
            ..Default::default()
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["map"], 1);
        assert_eq!(v["updates_us"][1], 900);
    }
}
