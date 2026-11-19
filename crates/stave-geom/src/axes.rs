//! Vectors read and written by axis, so no number stands for an axis.
//!
//! A part-frame vector ([`Vec3`] in millimetres, or [`DVec3`] in
//! nanometres for the float tests) has x along the part's length, y along
//! its width and z through its thickness. A world vector in nanometres
//! ([`DVec3`], as the clearance and collision checks keep them) has x
//! across, y up and z deep. Tables of other values per axis
//! are [`PerPartAxis`](stave_lang::placement::PerPartAxis) and
//! [`PerAxis`](stave_lang::placement::PerAxis).

use glam::{DVec3, Vec3};
use stave_lang::placement::{Axis, PartAxis};

/// A part-frame vector's coordinates, by part axis.
pub(crate) trait AlongPart {
    /// The coordinate type.
    type Scalar;

    /// The coordinate along `axis`.
    fn along(self, axis: PartAxis) -> Self::Scalar;

    /// Sets the coordinate along `axis`.
    fn set_along(&mut self, axis: PartAxis, value: Self::Scalar);
}

impl AlongPart for Vec3 {
    type Scalar = f32;

    fn along(self, axis: PartAxis) -> f32 {
        match axis {
            PartAxis::Length => self.x,
            PartAxis::Width => self.y,
            PartAxis::Thickness => self.z,
        }
    }

    fn set_along(&mut self, axis: PartAxis, value: f32) {
        match axis {
            PartAxis::Length => self.x = value,
            PartAxis::Width => self.y = value,
            PartAxis::Thickness => self.z = value,
        }
    }
}

impl AlongPart for DVec3 {
    type Scalar = f64;

    fn along(self, axis: PartAxis) -> f64 {
        match axis {
            PartAxis::Length => self.x,
            PartAxis::Width => self.y,
            PartAxis::Thickness => self.z,
        }
    }

    fn set_along(&mut self, axis: PartAxis, value: f64) {
        match axis {
            PartAxis::Length => self.x = value,
            PartAxis::Width => self.y = value,
            PartAxis::Thickness => self.z = value,
        }
    }
}

/// The part-frame vector `value` along `axis` and zero along the others.
pub(crate) fn on_part(axis: PartAxis, value: f32) -> Vec3 {
    let mut vector = Vec3::ZERO;
    vector.set_along(axis, value);
    vector
}

/// A world vector's coordinates, by world axis.
pub(crate) trait AlongWorld {
    /// The coordinate along `axis`.
    fn along_world(self, axis: Axis) -> f64;

    /// Sets the coordinate along `axis`.
    fn set_along_world(&mut self, axis: Axis, value: f64);
}

impl AlongWorld for DVec3 {
    fn along_world(self, axis: Axis) -> f64 {
        match axis {
            Axis::Across => self.x,
            Axis::Up => self.y,
            Axis::Deep => self.z,
        }
    }

    fn set_along_world(&mut self, axis: Axis, value: f64) {
        match axis {
            Axis::Across => self.x = value,
            Axis::Up => self.y = value,
            Axis::Deep => self.z = value,
        }
    }
}

/// The two part axes after `axis`, turning length → width → thickness →
/// length: `(u, v)` with `axis × u = v`, so a face across `axis` drawn
/// from `u` toward `v` faces along `axis`.
pub(crate) fn following(axis: PartAxis) -> [PartAxis; 2] {
    match axis {
        PartAxis::Length => [PartAxis::Width, PartAxis::Thickness],
        PartAxis::Width => [PartAxis::Thickness, PartAxis::Length],
        PartAxis::Thickness => [PartAxis::Length, PartAxis::Width],
    }
}

/// The part axis that is neither `first` nor `second`; `None` when the two
/// are the same axis.
pub(crate) fn third(first: PartAxis, second: PartAxis) -> Option<PartAxis> {
    PartAxis::ALL
        .into_iter()
        .find(|axis| *axis != first && *axis != second)
        .filter(|_| first != second)
}
