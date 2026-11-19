//! Checked numeric conversions for the SQLite and image boundaries.
//!
//! SQLite stores every integer as `i64`, while the domain values in this
//! crate are non-negative byte counts, dimensions, and row limits. These
//! helpers replace bare `as` casts so a corrupt or out-of-range value is
//! clamped to the representable extreme instead of silently wrapping.

/// Converts a count or index to the `i64` SQLite expects.
pub(crate) fn i64_from_usize(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Reads a stored `i64` back as a `usize` index or limit.
///
/// Negative values become `0`; values above the target's `usize` range
/// saturate to `usize::MAX`.
pub(crate) fn usize_from_i64(value: i64) -> usize {
    usize::try_from(value).unwrap_or(if value < 0 { 0 } else { usize::MAX })
}

/// Reads a stored `i64` as a non-negative byte count.
pub(crate) fn u64_from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// Stores a byte count that SQLite must accept as `i64`.
pub(crate) fn i64_from_u64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Stores a pixel dimension that SQLite must accept as `i64`.
pub(crate) fn i64_from_u32(value: u32) -> i64 {
    i64::from(value)
}

/// Reads a stored `i64` as an optional pixel dimension or people bound.
///
/// A missing value stays missing, and a value outside `u32` is treated as
/// unknown rather than clamped: for `people_min` a clamp upward and for
/// `people_max` a clamp downward would both invent a filter the stored row
/// does not support.
pub(crate) fn optional_u32_from_i64(value: Option<i64>) -> Option<u32> {
    value.and_then(|value| u32::try_from(value).ok())
}

/// Converts a stored `0.0..=1.0` progress value to the `f32` the model uses.
///
/// `f32::try_from(f64)` does not exist, so this is the one conversion that
/// needs an explicit allow. Corrupt values are clamped to the progress domain,
/// and non-finite values become `0.0` so the UI never displays NaN progress.
#[allow(
    clippy::as_conversions,
    reason = "no f32::try_from(f64); the value is clamped to the progress domain first"
)]
pub(crate) fn f32_from_f64(value: f64) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_in_range_values_without_change() {
        assert_eq!(i64_from_usize(0), 0);
        assert_eq!(i64_from_usize(200_000), 200_000);
        assert_eq!(usize_from_i64(42), 42);
        assert_eq!(u64_from_i64(1_234), 1_234);
        assert_eq!(i64_from_u64(94_249_600), 94_249_600);
        assert_eq!(i64_from_u32(1600), 1600);
        assert_eq!(optional_u32_from_i64(Some(1080)), Some(1080));
        assert_eq!(optional_u32_from_i64(None), None);
        assert_eq!(f32_from_f64(0.5), 0.5);
    }

    #[test]
    fn saturates_instead_of_wrapping_out_of_range_values() {
        assert_eq!(i64_from_usize(usize::MAX), i64::MAX);
        assert_eq!(i64_from_u64(u64::MAX), i64::MAX);
        assert_eq!(usize_from_i64(-1), 0);
        assert_eq!(
            usize_from_i64(i64::MAX),
            usize::try_from(i64::MAX).unwrap_or(usize::MAX)
        );
        assert_eq!(u64_from_i64(-1), 0);
    }

    #[test]
    fn out_of_range_optional_bounds_read_as_unknown() {
        assert_eq!(optional_u32_from_i64(Some(-1)), None);
        assert_eq!(optional_u32_from_i64(Some(5_000_000_000)), None);
    }

    #[test]
    fn progress_is_clamped_to_its_domain() {
        assert_eq!(f32_from_f64(-1.0), 0.0);
        assert_eq!(f32_from_f64(2.0), 1.0);
        assert_eq!(f32_from_f64(f64::NAN), 0.0);
        assert_eq!(f32_from_f64(f64::INFINITY), 0.0);
    }
}
