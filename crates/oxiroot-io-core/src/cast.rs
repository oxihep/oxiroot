//! Integer conversions between ROOT's on-disk types (`i32` sizes and counts,
//! `u32` byte counts, `i64`/`u64` offsets) and Rust's `usize`, without `as`.
//!
//! An `as` cast wraps a value out of range — a negative count read from a
//! damaged file became a huge `usize`, a length past `i32::MAX` a negative
//! size. [`SaturatingFrom`] clamps to the target's bound instead: a value
//! that cannot be represented never silently changes sign. Valid ROOT data
//! never reaches the bounds (its own fields are this narrow); the readers
//! validate what they decode separately.

/// A conversion that clamps an out-of-range value to the target type's
/// minimum or maximum.
///
/// ```
/// use oxiroot_io_core::cast::SaturatingFrom;
/// assert_eq!(usize::saturating_from(-3_i32), 0);
/// assert_eq!(i32::saturating_from(usize::MAX), i32::MAX);
/// assert_eq!(u32::saturating_from(7_usize), 7);
/// ```
pub trait SaturatingFrom<T>: Sized {
    /// `value`, or the nearest bound of `Self` when it does not fit.
    fn saturating_from(value: T) -> Self;
}

macro_rules! saturating {
    ($($from:ty => [$($to:ty),+]),+ $(,)?) => {$($(
        impl SaturatingFrom<$from> for $to {
            #[inline]
            fn saturating_from(value: $from) -> $to {
                // `try_from` fails only out of range; below zero (or the
                // target's minimum) clamps low, above clamps high.
                <$to>::try_from(value).unwrap_or_else(|_| {
                    #[allow(unused_comparisons, reason = "unsigned sources are never negative")]
                    let low = value < 0;
                    if low { <$to>::MIN } else { <$to>::MAX }
                })
            }
        }
    )+)+};
}

saturating!(
    i8 => [u8, u16, u32, u64, usize],
    i16 => [i8, u8, u16, u32, u64, usize],
    i32 => [i8, i16, u8, u16, u32, u64, usize],
    i64 => [i8, i16, i32, u8, u16, u32, u64, usize, isize],
    isize => [i8, i16, i32, u8, u16, u32, u64, usize],
    u8 => [i8],
    u16 => [i8, i16, u8],
    u32 => [i8, i16, i32, u8, u16, usize],
    u64 => [i8, i16, i32, i64, u8, u16, u32, usize, isize],
    usize => [i8, i16, i32, i64, u8, u16, u32, isize],
);
