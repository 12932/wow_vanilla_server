//! What the world benchmark (`src/bin/wvs-world-bench.rs`, the
//! `world-bench` feature) reads of each map: every update's wall and CPU
//! time, its steps' CPU time, the creatures awake after it, and the CPU
//! time of the events between updates. Off unless the feature is on, and
//! then a flag's load an update until [`enable`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ahash::AHashMap;
use parking_lot::Mutex;

use super::{Map, MapKey};

/// One update of one map.
#[derive(Debug, Clone)]
pub struct Sample {
    pub key: MapKey,
    pub wall: Duration,
    /// The update thread's CPU time over the update.
    pub cpu: Duration,
    /// Creatures awake at the update's end (seen by a player, or fighting).
    pub awake: u32,
    pub players: u32,
    /// Each step's CPU time.
    pub steps: Vec<(&'static str, Duration)>,
    /// The update thread's voluntary and involuntary context switches over
    /// the update.
    pub switches: (u64, u64),
}

/// What [`update_done`] needs of the update's start.
pub struct Start {
    pub cpu: Duration,
    pub switches: (u64, u64),
}

pub(super) fn start() -> Start {
    STEPS.with(|s| s.borrow_mut().clear());
    Start {
        cpu: thread_cpu(),
        switches: thread_switches(),
    }
}

static ON: AtomicBool = AtomicBool::new(false);
static SAMPLES: Mutex<Vec<Sample>> = Mutex::new(Vec::new());
static EVENTS: Mutex<Option<AHashMap<MapKey, (u64, Duration)>>> = Mutex::new(None);

thread_local! {
    static STEPS: std::cell::RefCell<Vec<(&'static str, Duration)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Start recording.
pub fn enable() {
    ON.store(true, Ordering::Relaxed);
}

/// Stop recording (samples and events taken until [`enable`] again).
pub fn disable() {
    ON.store(false, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// The samples since the last take, and the events' count and CPU time
/// by map.
pub fn take() -> (Vec<Sample>, AHashMap<MapKey, (u64, Duration)>) {
    let samples = std::mem::take(&mut *SAMPLES.lock());
    let events = EVENTS.lock().take().unwrap_or_default();
    (samples, events)
}

/// This thread's context switches: voluntary (it waited: on the disk, a
/// lock) and involuntary (the kernel ran something else).
pub fn thread_switches() -> (u64, u64) {
    // SAFETY: an all-zero rusage is valid, and getrusage writes into it.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `ru` is a valid rusage to write to.
    unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut ru) };
    (ru.ru_nvcsw as u64, ru.ru_nivcsw as u64)
}

/// This thread's CPU time.
pub fn thread_cpu() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec to write to.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

pub(super) fn step_done(name: &'static str, cpu: Duration) {
    STEPS.with(|s| s.borrow_mut().push((name, cpu)));
}

pub(super) fn update_done(map: &Map, wall: Duration, start: Start) {
    let cpu = thread_cpu().saturating_sub(start.cpu);
    let (v, i) = thread_switches();
    let switches = (
        v.saturating_sub(start.switches.0),
        i.saturating_sub(start.switches.1),
    );
    let awake = map
        .awake
        .iter()
        .filter(|g| {
            map.creature_by_guid
                .get(g)
                .and_then(|&k| map.creatures.get(k))
                .is_some_and(super::awake)
        })
        .count() as u32;
    let steps = STEPS.with(|s| std::mem::take(&mut *s.borrow_mut()));
    if wall >= SLOW_UPDATE {
        let mut top = steps.clone();
        top.sort_by_key(|s| std::cmp::Reverse(s.1));
        top.truncate(3);
        eprintln!(
            "{}: an update took {} ms ({} ms of CPU, {} waits, {} preemptions; {} players, {awake} creatures awake); its steps' CPU: {top:?}",
            map.key,
            wall.as_millis(),
            cpu.as_millis(),
            switches.0,
            switches.1,
            map.players.len()
        );
    }
    SAMPLES.lock().push(Sample {
        key: map.key,
        wall,
        cpu,
        awake,
        players: map.players.len() as u32,
        steps,
        switches,
    });
}

/// What a slow event or update is said to be: one over this, wall-clock.
const SLOW_EVENT: Duration = Duration::from_millis(200);
const SLOW_UPDATE: Duration = Duration::from_millis(1000);

/// [`start`] for an event: the steps' record is the update's.
pub(super) fn event_start() -> Start {
    Start {
        cpu: thread_cpu(),
        switches: thread_switches(),
    }
}

pub(super) fn event_done(key: MapKey, start: &Start, wall: Duration, kind: &str) {
    let cpu = thread_cpu().saturating_sub(start.cpu);
    if wall >= SLOW_EVENT {
        let (v, i) = thread_switches();
        eprintln!(
            "{key}: {kind} took {} ms ({} ms of CPU, {} waits, {} preemptions)",
            wall.as_millis(),
            cpu.as_millis(),
            v.saturating_sub(start.switches.0),
            i.saturating_sub(start.switches.1)
        );
    }
    let mut e = EVENTS.lock();
    let n = e.get_or_insert_with(AHashMap::new).entry(key).or_default();
    n.0 += 1;
    n.1 += cpu;
}
