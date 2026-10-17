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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_hears_of_the_first_panics_and_then_the_powers_of_two() {
        let logged: Vec<u64> = (1..=40).filter(|n| worth_logging(*n)).collect();
        assert_eq!(logged, [1, 2, 3, 4, 8, 16, 32]);
    }
}
