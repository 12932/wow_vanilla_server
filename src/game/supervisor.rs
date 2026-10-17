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
use std::time::{Duration, Instant};

#[cfg(test)]
use ahash::AHashMap;
#[cfg(test)]
use parking_lot::Mutex;
use tracing::error;

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
