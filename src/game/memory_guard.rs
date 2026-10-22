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

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::watch;

/// How often the memory is looked at.
const EVERY: Duration = Duration::from_millis(250);

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
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(EVERY);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let fraction = realm.config().memory_guard_fraction;
            let limit = (machine as f64 * fraction) as u64;
            let Some(rss) = crate::game::stats::rss_kb().map(|kb| kb * 1024) else {
                continue;
            };
            let was = on();
            let now = decide(was, rss, limit);
            if now == was {
                continue;
            }
            set(now);
            let mb = |b: u64| b / (1024 * 1024);
            if now {
                ENGAGED.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    "memory guard on: {} MB resident, past {} MB ({} of {} MB); \
                     the clients aren't read until it is under",
                    mb(rss),
                    mb(limit),
                    fraction,
                    mb(machine)
                );
            } else {
                tracing::info!(
                    "memory guard off: {} MB resident; the clients are read again",
                    mb(rss)
                );
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

    #[test]
    fn the_machine_reads() {
        assert!(machine_bytes().is_some_and(|b| b > 1 << 20));
    }
}
