//! Explicit numeric conversions at source storage boundaries.
/// Reinterpret a signed index word, retaining FBX's negative sentinels.
pub(crate) fn index_word(x: i32) -> u32 {
    u32::from_ne_bytes(x.to_ne_bytes())
}
pub(crate) fn signed_word(x: u32) -> i32 {
    i32::from_ne_bytes(x.to_ne_bytes())
}
/// The low word of a stored numeric bitfield (for example a class identifier).
pub(crate) fn low_word(x: i64) -> u32 {
    u32::from_le_bytes(x.to_le_bytes()[..4].try_into().unwrap())
}
/// Rust's saturating, truncating numeric conversion, with an explicit range check.
pub(crate) fn saturating_i64(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    if x >= 9_223_372_036_854_775_808.0 {
        return i64::MAX;
    }
    if x <= -9_223_372_036_854_775_808.0 {
        return i64::MIN;
    }
    // checked: finite x lies strictly between -2^63 and +2^63.
    #[allow(clippy::cast_possible_truncation)]
    {
        x as i64
    }
}
/// Round to the source's IEEE binary32 precision, including overflow.
pub(crate) fn round_f32(x: f64) -> f32 {
    if x.is_nan() {
        let sign = u32::try_from(x.to_bits() >> 63).unwrap() << 31;
        let payload = u32::try_from((x.to_bits() >> 29) & 0x7f_ffff).unwrap() | 0x40_0000;
        return f32::from_bits(sign | 0x7f80_0000 | payload);
    }
    // At the binary32 overflow midpoint, round-to-nearest-even produces infinity.
    if x.abs() >= 3.4028235677973366e38 {
        return f32::INFINITY.copysign(if x.is_sign_negative() { -1.0 } else { 1.0 });
    }
    // checked: x is finite, below the overflow midpoint in magnitude, so the
    // IEEE rounding below produces a finite binary32 value. Precision loss is intentional.
    #[allow(clippy::cast_possible_truncation)]
    {
        x as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_storage_boundaries_preserve_bits() {
        assert_eq!(index_word(-1), u32::MAX);
        assert_eq!(signed_word(0x8000_0000), i32::MIN);
        assert_eq!(low_word(0x1234_5678_9abc_def0), 0x9abc_def0);
        assert_eq!(saturating_i64(f64::INFINITY), i64::MAX);
        assert_eq!(saturating_i64(f64::NEG_INFINITY), i64::MIN);
        assert_eq!(saturating_i64(f64::NAN), 0);
        assert_eq!(saturating_i64(-1.75), -1);
        assert_eq!(round_f32(-0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(round_f32(f64::from(f32::MAX)), f32::MAX);
        let midpoint = 3.4028235677973366e38f64;
        assert_eq!(round_f32(f64::from_bits(midpoint.to_bits() - 1)), f32::MAX);
        assert_eq!(round_f32(midpoint), f32::INFINITY);
        assert_eq!(round_f32(-midpoint), f32::NEG_INFINITY);
        assert_eq!(round_f32(1.0 + 2.0f64.powi(-24)), 1.0);
        assert_eq!(round_f32(2.0f64.powi(-150)).to_bits(), 0);
        assert!(round_f32(f64::NAN).is_nan());
    }
}
