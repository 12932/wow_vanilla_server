//! The memory guard (`[network] memory_guard_fraction`): past a share of
//! the machine's memory, the server stops reading its clients until it is
//! back under. Each client's share of its map's mailbox is bounded
//! (`[network] max_queued_packets`) and so is each one's outbound queue
//! (`outbound_channel_bytes`), but together, for thousands, they are more
//! than a machine may have: this is the bound on the whole. A session that
//! reads nothing more of its client leaves it in the reader's queue and the
//! socket's (back-pressure, as a full share does): nothing is dropped, and
//! what the maps already have drains, which is what brings the memory down.
//!
//! A task looks at the process's resident memory four times a second; the
//! guard engages past the fraction and lets go once it is a twentieth under
//! (so it doesn't flap at the line), logging each.
//!
//! Reading less brings the memory down only when queues are what fills it.
//! When the steady state is over the line (a container's limit near the
//! game data, the navmesh and the allocator's own), it never comes down,
//! and a guard that held on would freeze the realm for good. So it holds
//! 30 s at most (`[network] memory_guard_most_held_secs`): then it says so
//! at `error`, lets go, and rests (a minute, `memory_guard_rest_secs`,
//! twice as long each time it gives up again, up to ten,
//! `memory_guard_rest_max_secs`) before it may engage again. The three are
//! read at every look, so a reload changes them at once.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::watch;

/// How often the memory is looked at.
const EVERY: Duration = Duration::from_millis(250);

/// The longest the guard holds the clients back at a time, by default.
pub const MOST_HELD: Duration = Duration::from_secs(30);
/// Its rest after giving up, the first time, and at most, by default.
pub const REST_MIN: Duration = Duration::from_secs(60);
pub const REST_MAX: Duration = Duration::from_secs(600);

/// How long the guard holds, and rests after giving up
/// (`[network] memory_guard_*_secs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timings {
    pub most_held: Duration,
    pub rest_min: Duration,
    pub rest_max: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Timings {
            most_held: MOST_HELD,
            rest_min: REST_MIN,
            rest_max: REST_MAX,
        }
    }
}

impl Timings {
    /// The rest after one of `last`: twice it, from the first rest up to
    /// the longest (never under the first, whatever the config says).
    fn next_rest(&self, last: Duration) -> Duration {
        (last * 2)
            .max(self.rest_min)
            .min(self.rest_max.max(self.rest_min))
    }
}

/// Times the guard has engaged since start-up (the stats file's).
pub static ENGAGED: AtomicU64 = AtomicU64::new(0);

fn channel() -> &'static (watch::Sender<bool>, watch::Receiver<bool>) {
    static CHANNEL: OnceLock<(watch::Sender<bool>, watch::Receiver<bool>)> = OnceLock::new();
    CHANNEL.get_or_init(|| watch::channel(false))
}

/// Whether the guard holds the sessions back now, and word of each change:
/// a session waits on it while it is on.
pub fn watch() -> watch::Receiver<bool> {
    channel().1.clone()
}

/// Whether the guard is on.
pub fn on() -> bool {
    *channel().1.borrow()
}

/// Turn the guard on or off (the task does, and a test may).
pub fn set(on: bool) {
    channel().0.send_if_modified(|v| {
        let changed = *v != on;
        *v = on;
        changed
    });
}

/// Whether the guard should be on: past `limit` it engages; once on, it
/// lets go a twentieth under.
pub fn decide(on: bool, rss: u64, limit: u64) -> bool {
    if limit == 0 {
        return false;
    }
    if on {
        rss > limit - limit / 20
    } else {
        rss > limit
    }
}

/// What the guard did at a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    On,
    Off,
    /// Held its longest and the memory never came down: off, and resting.
    GaveUp(Duration),
}

/// The guard's state from look to look.
#[derive(Debug, Default)]
pub struct Guard {
    on: bool,
    since: Option<Instant>,
    resting_until: Option<Instant>,
    rest: Duration,
}

impl Guard {
    pub fn on(&self) -> bool {
        self.on
    }

    /// A look at `now`: the resident memory, the limit, and the timings.
    pub fn look(&mut self, now: Instant, rss: u64, limit: u64, t: &Timings) -> Option<Change> {
        let resting = self.resting_until.is_some_and(|t| now < t);
        let want = !resting && decide(self.on, rss, limit);
        match (self.on, want) {
            (false, true) => {
                self.on = true;
                self.since = Some(now);
                Some(Change::On)
            }
            (true, true) if self.since.is_some_and(|s| now - s >= t.most_held) => {
                self.on = false;
                self.since = None;
                self.rest = t.next_rest(self.rest);
                self.resting_until = Some(now + self.rest);
                Some(Change::GaveUp(self.rest))
            }
            (true, false) => {
                // Under the line: holding back did bring it down.
                self.on = false;
                self.since = None;
                self.rest = Duration::ZERO;
                Some(Change::Off)
            }
            _ => None,
        }
    }
}

/// The machine's memory, in bytes: `/proc/meminfo`'s total, or the
/// process's cgroup's limit if that is less.
pub fn machine_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let total = meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse::<u64>().ok())?
        * 1024;
    // cgroup v2: the limit of the group the process is in, if it has one.
    let cgroup = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|c| {
            let path = c.lines().find_map(|l| l.strip_prefix("0::"))?.to_owned();
            std::fs::read_to_string(format!("/sys/fs/cgroup{path}/memory.max")).ok()
        })
        .and_then(|m| m.trim().parse::<u64>().ok());
    Some(cgroup.map_or(total, |c| c.min(total)))
}

/// Start the guard's task, with the realm's live fraction.
pub fn start(realm: &std::sync::Arc<crate::game::Realm>) {
    let Some(machine) = machine_bytes() else {
        tracing::warn!("memory guard: the machine's memory can't be read; off");
        return;
    };
    let realm = realm.clone();
    if let Some(rss) = crate::game::stats::rss_kb().map(|kb| kb * 1024) {
        let limit = (machine as f64 * realm.config().memory_guard_fraction) as u64;
        if limit > 0 && rss > limit / 10 * 9 {
            tracing::warn!(
                "memory guard: {} MB resident at start-up, near its line of {} MB: it may \
                 engage with nothing to drain",
                rss / (1024 * 1024),
                limit / (1024 * 1024)
            );
        }
    }
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(EVERY);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut guard = Guard::default();
        loop {
            ticker.tick().await;
            let cfg = realm.config();
            let fraction = cfg.memory_guard_fraction;
            let timings = cfg.memory_guard_timings;
            let limit = (machine as f64 * fraction) as u64;
            let Some(rss) = crate::game::stats::rss_kb().map(|kb| kb * 1024) else {
                continue;
            };
            let Some(change) = guard.look(Instant::now(), rss, limit, &timings) else {
                continue;
            };
            set(guard.on());
            let mb = |b: u64| b / (1024 * 1024);
            match change {
                Change::On => {
                    ENGAGED.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        "memory guard on: {} MB resident, past {} MB ({} of {} MB); \
                         the clients aren't read until it is under",
                        mb(rss),
                        mb(limit),
                        fraction,
                        mb(machine)
                    );
                }
                Change::Off => tracing::info!(
                    "memory guard off: {} MB resident; the clients are read again",
                    mb(rss)
                ),
                Change::GaveUp(rest) => tracing::error!(
                    "memory guard: {} MB resident after holding the clients back {} s, still \
                     past {} MB: reading less doesn't bring it down. The clients are read \
                     again; it rests {} s. Raise the memory or [network] memory_guard_fraction",
                    mb(rss),
                    timings.most_held.as_secs(),
                    mb(limit),
                    rest.as_secs()
                ),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_engages_past_the_limit_and_lets_go_a_twentieth_under() {
        let limit = 1000;
        assert!(!decide(false, 1000, limit));
        assert!(decide(false, 1001, limit));
        assert!(decide(true, 951, limit), "held on, near the line");
        assert!(!decide(true, 950, limit), "let go under");
        assert!(!decide(false, u64::MAX, 0), "0 is off");
    }

    /// Held as long as it may with the memory never down, the guard lets
    /// go and rests, longer each time; one that brought it down rests not
    /// at all. (It held on for good: a realm whose steady state was over
    /// the line froze.)
    #[test]
    fn a_guard_that_brings_nothing_down_lets_go_and_rests() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut g = Guard::default();
        let t = Timings::default();
        assert_eq!(g.look(at(0), 2000, 1000, &t), Some(Change::On));
        assert_eq!(g.look(at(29), 2000, 1000, &t), None, "held");
        assert_eq!(
            g.look(at(30), 2000, 1000, &t),
            Some(Change::GaveUp(REST_MIN))
        );
        assert!(!g.on());
        assert_eq!(g.look(at(89), 2000, 1000, &t), None, "resting");
        assert_eq!(g.look(at(90), 2000, 1000, &t), Some(Change::On));
        assert_eq!(
            g.look(at(120), 2000, 1000, &t),
            Some(Change::GaveUp(REST_MIN * 2))
        );
        assert_eq!(g.look(at(240), 2000, 1000, &t), Some(Change::On));
        assert_eq!(
            g.look(at(250), 900, 1000, &t),
            Some(Change::Off),
            "brought down"
        );
        assert_eq!(g.look(at(251), 2000, 1000, &t), Some(Change::On));
        assert_eq!(
            g.look(at(281), 2000, 1000, &t),
            Some(Change::GaveUp(REST_MIN))
        );
    }

    /// The timings are the config's (`[network] memory_guard_*_secs`), read
    /// at each look: a guard told to hold 5 s and rest 10, doubling to 15,
    /// does; and a change between looks applies at the next.
    #[test]
    fn a_guard_holds_and_rests_as_long_as_its_settings_say() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let secs = Duration::from_secs;
        let t = Timings {
            most_held: secs(5),
            rest_min: secs(10),
            rest_max: secs(15),
        };
        let mut g = Guard::default();
        assert_eq!(g.look(at(0), 2000, 1000, &t), Some(Change::On));
        assert_eq!(g.look(at(4), 2000, 1000, &t), None, "held");
        assert_eq!(
            g.look(at(5), 2000, 1000, &t),
            Some(Change::GaveUp(secs(10)))
        );
        assert_eq!(g.look(at(14), 2000, 1000, &t), None, "resting");
        assert_eq!(g.look(at(15), 2000, 1000, &t), Some(Change::On));
        assert_eq!(
            g.look(at(20), 2000, 1000, &t),
            Some(Change::GaveUp(secs(15))),
            "doubled, up to the longest"
        );
        assert_eq!(g.look(at(35), 2000, 1000, &t), Some(Change::On));
        assert_eq!(
            g.look(at(40), 2000, 1000, &t),
            Some(Change::GaveUp(secs(15)))
        );
        // A reload: hold 20 s now.
        let longer = Timings {
            most_held: secs(20),
            ..t
        };
        assert_eq!(g.look(at(55), 2000, 1000, &longer), Some(Change::On));
        assert_eq!(
            g.look(at(70), 2000, 1000, &longer),
            None,
            "held past the old 5"
        );
        assert_eq!(
            g.look(at(75), 2000, 1000, &longer),
            Some(Change::GaveUp(secs(15)))
        );
        // A longest rest under the first is the first.
        let odd = Timings {
            rest_max: secs(1),
            ..t
        };
        assert_eq!(odd.next_rest(Duration::ZERO), secs(10));
        assert_eq!(odd.next_rest(secs(10)), secs(10));
    }

    #[test]
    fn the_machine_reads() {
        assert!(machine_bytes().is_some_and(|b| b > 1 << 20));
    }
}
