//! The viewer's number conversions, in one place: GTK hands over `f64`
//! and `i32`, the GPU wants `f32` and `u32`. Exact values were decided in
//! nanometres long before; here only screen-sized numbers pass, far inside
//! every type's range, so at worst precision beyond a pixel is lost.

/// An `f64` as the nearest `f32` (the GPU boundary; 24 significant bits).
pub fn f32_of(value: f64) -> f32 {
    // Rust defines this as rounding to nearest; the one float narrowing in
    // the viewer.
    #[allow(clippy::cast_possible_truncation)]
    let narrowed = value as f32;
    narrowed
}

/// A widget coordinate or size as `f32`.
pub fn f32_of_int(value: i32) -> f32 {
    f32_of(f64::from(value))
}

/// A pixel count as `f32`.
pub fn f32_of_u32(value: u32) -> f32 {
    f32_of(f64::from(value))
}

/// A small index as `f32` (exact below 2^24).
pub fn f32_of_index(value: usize) -> f32 {
    f32_of(f64::from(u32::try_from(value).unwrap_or(u32::MAX)))
}

/// A device pixel count from a scaled size: rounded and kept within
/// `1..=16384`.
pub fn pixels(value: f64) -> u32 {
    let clamped = value.round().clamp(1.0, 16384.0);
    stave_geom::render::integer_from_f64(clamped)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(1)
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
        assert_eq!(f32_of_int(1600), 1600.0);
        assert_eq!(f32_of_u32(1000), 1000.0);
        assert_eq!(pixels(1599.6), 1600);
        assert_eq!(pixels(-3.0), 1);
        assert_eq!(pixels(1e9), 16384);
        assert_eq!(bytes(64), 64);
    }
}
