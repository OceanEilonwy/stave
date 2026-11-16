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
    // The only narrowing cast at the boundary: f64 millimetres to f32.
    length.to_mm_f64() as f32
}

/// An angle in radians.
pub fn radians(angle: Angle) -> f32 {
    angle.to_radians_f64() as f32
}

/// A length in nanometres, for collision tests.
pub fn nm(length: Length) -> f64 {
    // Exact for any length below 2^53 nm (about 9000 km).
    length.nm() as f64
}

/// A distance in nanometres back to a length, rounded to the nearest
/// nanometre (saturating far outside any design).
pub fn length_from_nm(nm: f64) -> Length {
    // The only float-to-integer cast: rounding a measured distance once.
    Length::from_nm(nm.round() as i64)
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
