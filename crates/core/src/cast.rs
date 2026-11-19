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
pub(crate) fn usize_from_i64(value: i64) -> usize {
    usize::try_from(value).unwrap_or(0)
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

/// Reads a stored `i64` back as a pixel dimension.
pub(crate) fn u32_from_i64(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

/// Converts a stored `0.0..=1.0` progress value to the `f32` the model uses.
///
/// `f32::try_from(f64)` does not exist, so this is the one conversion that
/// needs an explicit allow. Progress is written by this crate as a value in
/// `0.0..=1.0`; a value outside that domain still saturates correctly
/// because every `f64` in range converts to a finite `f32`.
#[allow(
    clippy::as_conversions,
    reason = "no f32::try_from(f64); stored progress is a 0.0..=1.0 ratio"
)]
pub(crate) fn f32_from_f64(value: f64) -> f32 {
    value as f32
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
        assert_eq!(u32_from_i64(1080), 1080);
        assert_eq!(f32_from_f64(0.5), 0.5);
    }

    #[test]
    fn saturates_instead_of_wrapping_out_of_range_values() {
        assert_eq!(i64_from_usize(usize::MAX), i64::MAX);
        assert_eq!(i64_from_u64(u64::MAX), i64::MAX);
        assert_eq!(usize_from_i64(-1), 0);
        assert_eq!(u64_from_i64(-1), 0);
        assert_eq!(u32_from_i64(-1), 0);
    }
}
