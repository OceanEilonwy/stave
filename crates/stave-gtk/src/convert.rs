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

/// A device pixel count from a scaled size: rounded and kept within
/// `1..=16384`.
pub fn pixels(value: f64) -> u32 {
    let clamped = value.round().clamp(1.0, 16384.0);
    stave_geom::render::integer_from_f64(clamped)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(1)
}

/// A pointer coordinate as the whole widget pixel it falls in.
pub fn int_of(value: f64) -> i32 {
    stave_geom::render::integer_from_f64(value.floor().clamp(-1e6, 1e6))
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

/// A small `f32` (a percentage, a slider's reading) as the nearest whole
/// number; 0 when it isn't a number or is out of `i32`'s range.
pub fn int_of_f32(value: f32) -> i32 {
    stave_geom::render::integer_from_f64(f64::from(value).round())
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_numbers_convert_exactly() {
        assert_eq!(f32_of_int(1600), 1600.0);
        assert_eq!(pixels(1599.6), 1600);
        assert_eq!(pixels(-3.0), 1);
        assert_eq!(pixels(1e9), 16384);
        assert_eq!(int_of_f32(99.6), 100);
        assert_eq!(int_of_f32(f32::NAN), 0);
    }
}
