//! The one place exact values become floats: lengths to `f32`
//! millimetres, angles to `f32` radians, world points and directions to
//! the render frame (see the crate docs); and, for collision tests of
//! round shapes and turned assemblies, lengths to `f64` nanometres and
//! back.
//!
//! Rendering needs only about a micrometre of precision over a few metres,
//! well inside `f32`. Collision tests keep `f64` nanometres: a few metres
//! are about 2^32 nm, far inside the 2^53 an `f64` holds exactly.

use glam::Vec3;
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length};
use stave_model::solved::{Point, Size3};

/// A length in millimetres.
pub fn mm(length: Length) -> f32 {
    gpu(length.to_mm_f64())
}

/// An angle in radians.
pub fn radians(angle: Angle) -> f32 {
    gpu(angle.to_radians_f64())
}

/// The GPU boundary: an `f64` narrowed to the nearest `f32`, the one
/// float narrowing in Stave. An `f32` keeps 24 significant bits, so a
/// length of a few metres (in millimetres) is held to about 0.1µm and an
/// angle to about 1e-7 rad, far finer than a screen shows; non-finite
/// values stay non-finite, and values beyond `f32`'s range (never a
/// design's) become infinite. Everything exact has already been decided
/// in nanometres before this point.
fn gpu(value: f64) -> f32 {
    // Rust defines this cast as rounding to nearest, ties to even.
    #[allow(clippy::cast_possible_truncation)]
    let narrowed = value as f32;
    narrowed
}

/// A length in nanometres, for collision tests.
///
/// Exact for every length below 2^53 nm (about 9000 km), so for anything
/// a design holds; beyond that the nearest `f64` is returned. Built from
/// the two 32-bit halves, each converted losslessly, so no integer is ever
/// truncated: only the final sum rounds, and only past 2^53.
pub fn nm(length: Length) -> f64 {
    let value = length.nm();
    // `value >> 32` always fits an i32; the low half always fits a u32.
    let high = i32::try_from(value >> 32).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX });
    let low = u32::try_from(value & 0xFFFF_FFFF).unwrap_or(0);
    f64::from(high) * 4_294_967_296.0 + f64::from(low)
}

/// A distance in nanometres back to a length, rounded to the nearest
/// nanometre. `None` for a value that isn't finite or lies outside what a
/// [`Length`] holds (about ±9.2e18 nm); a collision test never makes one,
/// and callers treat it as nothing found rather than guess.
pub fn length_from_nm(nm: f64) -> Option<Length> {
    integer_from_f64(nm.round()).map(Length::from_nm)
}

/// An angle in radians, rounded down to the micro-degree; `None` when it
/// isn't finite.
pub fn angle_from_radians_floor(radians: f64) -> Option<Angle> {
    integer_from_f64((radians.to_degrees() * 1_000_000.0).floor()).map(Angle::from_microdeg)
}

/// The `i64` an integral `f64` holds, decoded from its bits (exact, with
/// no cast); `None` when it isn't finite or doesn't fit.
pub fn integer_from_f64(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some(0);
    }
    let bits = value.to_bits();
    let negative = bits >> 63 == 1;
    let exponent = i64::try_from((bits >> 52) & 0x7FF).ok()? - 1075;
    let mantissa = u128::from((bits & ((1u64 << 52) - 1)) | (1u64 << 52));
    let magnitude = if exponent >= 0 {
        if exponent > 64 {
            return None;
        }
        mantissa.checked_shl(u32::try_from(exponent).ok()?)?
    } else {
        // An integral value: the bits shifted out are zero.
        mantissa >> u32::try_from(-exponent).ok()?.min(127)
    };
    if negative {
        let magnitude = i128::try_from(magnitude).ok()?;
        i64::try_from(-magnitude).ok()
    } else {
        i64::try_from(magnitude).ok()
    }
}

/// An angle in radians, for collision tests.
pub fn radians_f64(angle: Angle) -> f64 {
    angle.to_radians_f64()
}

/// A world point in the render frame.
pub fn point(point: Point) -> Vec3 {
    Vec3::new(mm(point.across), mm(point.up), -mm(point.deep))
}

/// The unit vector of a world direction in the render frame.
pub fn direction(direction: Direction) -> Vec3 {
    match direction {
        Direction::Right => Vec3::X,
        Direction::Left => Vec3::NEG_X,
        Direction::Up => Vec3::Y,
        Direction::Down => Vec3::NEG_Y,
        Direction::Front => Vec3::Z,
        Direction::Back => Vec3::NEG_Z,
    }
}

/// The unit vector of a world axis's positive direction.
pub fn axis(axis: Axis) -> Vec3 {
    match axis {
        Axis::Across => Vec3::X,
        Axis::Up => Vec3::Y,
        Axis::Deep => Vec3::NEG_Z,
    }
}

/// A size along a part's own axes, as `(length, width, thickness)` in
/// millimetres.
pub fn size(size: Size3) -> Vec3 {
    Vec3::new(mm(size.length), mm(size.width), mm(size.thickness))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nanometres_go_to_floats_and_back_exactly() {
        for nm in [
            0,
            1,
            -1,
            50_000,
            2_000_000_000,
            -1_234_567_891,
            1 << 52,
            -(1 << 52),
        ] {
            let length = Length::from_nm(nm);
            assert_eq!(super::nm(length), nm as f64);
            assert_eq!(length_from_nm(super::nm(length)), Some(length));
        }
        assert_eq!(length_from_nm(2.4), Some(Length::from_nm(2)));
        assert_eq!(length_from_nm(-2.6), Some(Length::from_nm(-3)));
        assert_eq!(length_from_nm(f64::NAN), None);
        assert_eq!(length_from_nm(f64::INFINITY), None);
        assert_eq!(length_from_nm(1e19), None);
        assert_eq!(length_from_nm(-9.3e18), None);
        assert_eq!(
            length_from_nm(-9_223_372_036_854_775_808.0),
            Some(Length::from_nm(i64::MIN))
        );
    }

    #[test]
    fn floats_narrow_to_the_nearest_f32() {
        assert_eq!(gpu(1.5), 1.5_f32);
        assert_eq!(mm(Length::from_nm(19_050_000)), 19.05_f32);
        assert!((gpu(1_234.000_000_1) - 1234.0).abs() < 1e-3);
        assert!(gpu(f64::NAN).is_nan());
        assert_eq!(gpu(1e300), f32::INFINITY);
    }

    #[test]
    fn deep_runs_toward_minus_z() {
        let p = Point {
            across: Length::from_mm(1),
            up: Length::from_mm(2),
            deep: Length::from_mm(3),
        };
        assert_eq!(point(p), Vec3::new(1.0, 2.0, -3.0));
        assert_eq!(direction(Direction::Back), axis(Axis::Deep));
    }
}
