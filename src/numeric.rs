//! Numeric helpers that aren't a one-liner in std.
//!
//! Most numeric boundary conversions in the project should use stdlib:
//! - Saturating int narrowing: `i32::try_from(v).unwrap_or(i32::MAX)`.
//! - `f32` -> `uN` clamped to [0, uN::MAX]: bare `as` already saturates (Rust
//!   1.45+) and `NaN as uN == 0`. Just guard negative input with `.max(0.0)`.
//!
//! Helpers below cover the cases that *aren't* idiomatic one-liners.

/// Convert a `u64` random value to a uniformly-distributed `f32` in `[0, 1)`.
///
/// Naive `r as f32 / u64::MAX as f32` is buggy: `u64::MAX as f32` rounds to a
/// power of two and the top bit of `r` silently collapses. f32 has only 24
/// bits of mantissa precision, so we extract the top 24 bits of the random
/// value and divide by `2^24`. Result is unbiased to within float precision.
pub fn rand_unit_f32(r: u64) -> f32 {
    const SCALE: f32 = (1u32 << 24) as f32;
    let mantissa_bits = (r >> 40) as u32; // top 24 bits
    mantissa_bits as f32 / SCALE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rand_unit_f32_zero() {
        assert_eq!(rand_unit_f32(0), 0.0);
    }

    #[test]
    fn rand_unit_f32_max_below_one() {
        // The top 24 bits of u64::MAX produce (2^24 - 1) / 2^24, which is the
        // largest representable value strictly less than 1.0 from this scheme.
        let r = rand_unit_f32(u64::MAX);
        assert!(r < 1.0, "rand_unit_f32 should never return 1.0, got {r}");
        assert!(r > 0.99);
    }

    #[test]
    fn rand_unit_f32_preserves_top_bit() {
        // Naive `r as f32 / u64::MAX as f32` would round half the input space
        // to exactly 1.0 because the top bit of `r` collapses. Make sure our
        // implementation distinguishes the high half of u64 from the low half.
        let low = rand_unit_f32(1u64 << 62);
        let high = rand_unit_f32(1u64 << 63);
        assert!(high > low, "high bit must affect output: low={low} high={high}");
    }
}
