//! The renderer's number conversions, in one place: the GPU wants `f32`
//! and `u64` sizes. Only screen-sized numbers and small counts pass, far
//! inside every type's range.

/// A pixel count as `f32` (exact below 2^24, which every view is).
pub fn f32_of_u32(value: u32) -> f32 {
    // Rust defines this as rounding to nearest; the one float narrowing in
    // the renderer.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rounds to nearest, exact for pixel counts"
    )]
    let narrowed = f64::from(value) as f32;
    narrowed
}

/// A small index as `f32` (exact below 2^24).
pub fn f32_of_index(value: usize) -> f32 {
    f32_of_u32(u32::try_from(value).unwrap_or(u32::MAX))
}

/// A byte size for a GPU buffer.
pub fn bytes(size: usize) -> u64 {
    u64::try_from(size).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_numbers_convert_exactly() {
        assert_eq!(f32_of_u32(1000), 1000.0);
        assert_eq!(f32_of_index(96), 96.0);
        assert_eq!(bytes(64), 64);
    }
}
