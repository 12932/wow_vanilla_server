//! Some of Erlang/OTP's reliability, where it is cheap (docs/RELIABILITY.md):
//! a panic costs the step it happened in, not the task it happened on; a
//! loop that panics backs off and tries again; the panic is logged once,
//! with where it happened; and a watchdog says when a map has hung or
//! can't keep up.
//!
//! What a map does about its own panics (an entity's, an update's, its
//! restart) is in `map/supervise.rs`.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use ahash::AHashMap;
use parking_lot::Mutex;
use tracing::{error, warn};

use crate::game::Realm;

/// What a panic said: its message, if it was a string.
pub fn message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_owned())
}

/// Run `f`, catching a panic: what it said, if it did. Free when nothing
/// panics.
pub fn catch<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|e| message(&*e))
}

/// Whether the `n`th time something happened is worth a line in the log:
/// the first three, then the powers of two, so a panic every tick doesn't
/// flood it.
pub fn worth_logging(n: u64) -> bool {
    n <= 3 || n.is_power_of_two()
}

/// A step of a long-running loop, under watch: a panic is logged and the
/// step rests a while before it is tried again, twice as long each time
/// it panics again, up to [`Backoff::MAX`]. A run that doesn't panic
/// clears the rest. OTP's restart of a child, with its intensity spread
/// out rather than given up on.
#[derive(Debug, Default)]
pub struct Backoff {
    rest: Duration,
    until: Option<Instant>,
    pub panics: u64,
}

impl Backoff {
    pub const MIN: Duration = Duration::from_secs(1);
    pub const MAX: Duration = Duration::from_secs(60);

    /// Run `f` unless it is resting; false if it panicked or rested.
    pub fn run(&mut self, what: &str, now: Instant, f: impl FnOnce()) -> bool {
        if self.until.is_some_and(|t| now < t) {
            return false;
        }
        match catch(f) {
            Ok(()) => {
                self.rest = Duration::ZERO;
                self.until = None;
                true
            }
            Err(e) => {
                self.panics += 1;
                self.rest = (self.rest * 2).clamp(Self::MIN, Self::MAX);
                self.until = Some(now + self.rest);
                if worth_logging(self.panics) {
                    error!(
                        "{what} panicked: {e}; trying again in {} s ({} times)",
                        self.rest.as_secs(),
                        self.panics
                    );
                }
                false
            }
        }
    }

    /// Whether it is resting at `now`.
    pub fn resting(&self, now: Instant) -> bool {
        self.until.is_some_and(|t| now < t)
    }
}

/// Run a long-lived task, starting it again whenever it panics: OTP's
/// permanent child. The first restart is at once, and each one after rests
/// twice as long as the last, up to [`Backoff::MAX`]. It ends when the
/// task returns.
pub async fn supervise<F, Fut>(what: &str, mut start: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let mut rest = Duration::ZERO;
    loop {
        match tokio::spawn(start()).await {
            Ok(()) => return,
            Err(e) if e.is_panic() => {
                let e = message(&*e.into_panic());
                error!(
                    "{what} panicked: {e}; starting it again in {} s",
                    rest.as_secs()
                );
                tokio::time::sleep(rest).await;
                rest = (rest * 2).clamp(Backoff::MIN, Backoff::MAX);
            }
            Err(_) => return,
        }
    }
}

/// Panics seen by the hook, by where they happened.
static SEEN: LazyLock<Mutex<AHashMap<String, u64>>> = LazyLock::new(Default::default);

/// Log every panic once, through tracing: where it happened, what it said,
/// and (the first time at that place) a backtrace. The default hook's
/// lines on stderr are lost to a log; this puts them with the rest, and a
/// panic every tick costs a backtrace once.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let at = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "an unknown place".to_owned());
        let what = message(info.payload());
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("unnamed");
        let n = {
            let mut seen = SEEN.lock();
            let n = seen.entry(at.clone()).or_insert(0);
            *n += 1;
            *n
        };
        if n == 1 {
            let trace = std::backtrace::Backtrace::force_capture();
            error!("panic in thread {thread} at {at}: {what}\n{trace}");
        } else if worth_logging(n) {
            error!(
                "panic in thread {thread} at {at}: {what} ({n} times there; its backtrace is at the first)"
            );
        }
    }));
}

/// How many panics the hook has seen at `at` (`file:line:column`).
pub fn seen(at: &str) -> u64 {
    SEEN.lock().get(at).copied().unwrap_or(0)
}

/// What a map's actor tells the watchdog, at each turn of its loop: that it
/// is alive, and how many events wait for it. Two relaxed stores a turn.
#[derive(Debug)]
pub struct Pulse {
    /// When the actor last came round its loop, in ms since start-up.
    beat: AtomicU32,
    queued: AtomicUsize,
}

impl Default for Pulse {
    fn default() -> Self {
        Self {
            beat: AtomicU32::new(crate::game::uptime_ms()),
            queued: AtomicUsize::new(0),
        }
    }
}

impl Pulse {
    pub fn beat(&self, queued: usize) {
        self.beat.store(crate::game::uptime_ms(), Ordering::Relaxed);
        self.queued.store(queued, Ordering::Relaxed);
    }

    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }
}

/// When the watchdog speaks up.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A map that hasn't come round its loop in this long is hung.
    pub hung_after: Duration,
    /// A map with this many events waiting can't keep up.
    pub mailbox: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            hung_after: Duration::from_secs(10),
            mailbox: 10_000,
        }
    }
}

/// What the watchdog found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alarm {
    /// The map has been in one event or update this long.
    Hung(crate::game::map::MapKey, Duration),
    /// It came back.
    Back(crate::game::map::MapKey),
    /// This many events wait for it.
    Swamped(crate::game::map::MapKey, usize),
    /// Its mailbox is back down.
    Drained(crate::game::map::MapKey),
}

/// How often the watchdog looks.
pub const WATCH_EVERY: Duration = Duration::from_secs(1);

/// What the watchdog has said of each map: that it hung, that it can't
/// keep up.
#[derive(Debug, Default)]
pub struct Watched(AHashMap<crate::game::map::MapKey, (bool, bool)>);

/// Look at every map's pulse once, logging what has changed since the
/// last look: a map that hangs or recovers, a mailbox that grows past the
/// limit or drains.
pub fn check(realm: &Realm, limits: Limits, watched: &mut Watched) -> Vec<Alarm> {
    let now = crate::game::uptime_ms();
    let mut alarms = Vec::new();
    for map in realm.maps.all() {
        let pulse = map.pulse();
        let since = Duration::from_millis(u64::from(
            now.saturating_sub(pulse.beat.load(Ordering::Relaxed)),
        ));
        let (hung, swamped) = watched.0.entry(map.key).or_default();
        if since >= limits.hung_after {
            if !std::mem::replace(hung, true) {
                error!(
                    "{} has been in one event or update for {} s: it has hung (an endless loop?), and its players with it",
                    map.key,
                    since.as_secs()
                );
                alarms.push(Alarm::Hung(map.key, since));
            }
        } else if std::mem::replace(hung, false) {
            warn!("{} is going again", map.key);
            alarms.push(Alarm::Back(map.key));
        }
        let queued = pulse.queued();
        if queued >= limits.mailbox {
            if !std::mem::replace(swamped, true) {
                error!(
                    "{} has {queued} events waiting: it can't keep up with what it is sent",
                    map.key
                );
                alarms.push(Alarm::Swamped(map.key, queued));
            }
        } else if queued < limits.mailbox / 10 && std::mem::replace(swamped, false) {
            warn!("{} has caught up ({queued} events waiting)", map.key);
            alarms.push(Alarm::Drained(map.key));
        }
    }
    alarms
}

/// Start the watchdog, while the realm lasts. It runs on a thread of its
/// own, so a map that has hung a runtime's worker can't hang it too.
pub fn start_watchdog(realm: &Arc<Realm>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let weak = Arc::downgrade(realm);
    let spawned = std::thread::Builder::new()
        .name("watchdog".into())
        .spawn(move || {
            let mut backoff = Backoff::default();
            let mut watched = Watched::default();
            loop {
                std::thread::sleep(WATCH_EVERY);
                let Some(realm) = weak.upgrade() else {
                    return;
                };
                backoff.run("the watchdog", Instant::now(), || {
                    check(&realm, Limits::default(), &mut watched);
                });
            }
        });
    if let Err(e) = spawned {
        warn!("no watchdog: {e}");
    }
}

/// Failures a test arms by name: the next `n` times something checks for
/// one, it panics.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct Faults {
    armed: Mutex<AHashMap<String, u32>>,
    /// Times each name was checked, armed or not.
    checked: Mutex<AHashMap<String, u32>>,
}

#[cfg(test)]
impl Faults {
    /// The next `times` checks of `what` panic (`u32::MAX` for always).
    pub fn arm(&self, what: impl Into<String>, times: u32) {
        self.armed.lock().insert(what.into(), times);
    }

    pub fn disarm(&self, what: &str) {
        self.armed.lock().remove(what);
    }

    /// Panic if `what` is armed.
    pub fn check(&self, what: &str) {
        *self.checked.lock().entry(what.to_owned()).or_default() += 1;
        let fire = {
            let mut armed = self.armed.lock();
            match armed.get_mut(what) {
                Some(n) if *n > 0 => {
                    *n = n.saturating_sub(u32::from(*n != u32::MAX));
                    true
                }
                _ => false,
            }
        };
        if fire {
            panic!("a test's fault: {what}");
        }
    }

    /// Whether anything is armed: a check that costs nothing otherwise.
    pub fn any(&self) -> bool {
        !self.armed.lock().is_empty()
    }

    /// How many times `what` has been checked.
    pub fn checked(&self, what: &str) -> u32 {
        self.checked.lock().get(what).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_that_panics_rests_longer_each_time_and_a_good_run_clears_it() {
        let mut b = Backoff::default();
        let t0 = Instant::now();
        let mut ran = 0;
        assert!(!b.run("a step", t0, || panic!("one")));
        assert!(b.resting(t0));
        // Resting: not run.
        assert!(!b.run("a step", t0 + Duration::from_millis(500), || ran += 1));
        assert_eq!(ran, 0);
        // Tried again after a second, and it panics again: two seconds.
        let t1 = t0 + Backoff::MIN;
        assert!(!b.run("a step", t1, || panic!("two")));
        assert!(b.resting(t1 + Duration::from_millis(1500)));
        let t2 = t1 + Backoff::MIN * 2;
        assert!(b.run("a step", t2, || ran += 1));
        assert_eq!((ran, b.panics), (1, 2));
        // Clear again: the next panic rests the least.
        assert!(!b.run("a step", t2, || panic!("three")));
        assert!(!b.resting(t2 + Backoff::MIN));
    }

    #[test]
    fn the_log_hears_of_the_first_panics_and_then_the_powers_of_two() {
        let logged: Vec<u64> = (1..=40).filter(|n| worth_logging(*n)).collect();
        assert_eq!(logged, [1, 2, 3, 4, 8, 16, 32]);
    }
}
