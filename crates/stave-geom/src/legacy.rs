//! The old motion contract, read only by [`crate::clearance::closest_for`]
//! until its callers move to the assembly's mechanism (work pack
//! "generalise kinds", hand-off H2). This is the only module of
//! `stave-geom` that reads a [`Motion`]'s variants; it is deleted with
//! them.

use stave_lang::placement::Axis;
use stave_lang::units::{Angle, Length};
use stave_model::solved::{Motion, PoseValue};

use crate::collide::{Placement, direction_offset, turn_about};
use crate::linkage::LinkagePath;
use crate::render;

/// The world axis a turning motion turns about; `None` for a travel.
pub(crate) fn turning_axis(motion: &Motion) -> Option<Axis> {
    match motion {
        Motion::Turn { pivot, .. } => Some(pivot.axis),
        Motion::Linkage { linkage } => Some(linkage.frame().axis()),
        Motion::Travel { .. } => None,
    }
}

/// How far a travelling motion goes, and which way; `None` for a turn.
pub(crate) fn travel(motion: &Motion) -> Option<(stave_lang::placement::Direction, Length)> {
    match *motion {
        Motion::Travel { toward, travel } => Some((toward, travel)),
        Motion::Turn { .. } | Motion::Linkage { .. } => None,
    }
}

/// The placement an old motion gives its assembly at a pose value.
pub(crate) fn placement(motion: &Motion, value: PoseValue) -> Placement {
    match *motion {
        Motion::Travel { toward, travel } => {
            let amount = match value {
                PoseValue::Open => travel,
                PoseValue::Travel(length) => length,
                PoseValue::Closed | PoseValue::Angle(_) => Length::ZERO,
            };
            Placement::Exact(direction_offset(toward, amount))
        }
        Motion::Turn {
            pivot,
            opening_angle,
        } => match turned(value, opening_angle) {
            Some(angle) => Placement::Turned(turn_about(&pivot, angle)),
            None => Placement::REST,
        },
        Motion::Linkage { linkage } => {
            let Some(angle) = turned(value, linkage.opening_angle()) else {
                return Placement::REST;
            };
            let path = LinkagePath::new(&linkage);
            // Past the checked range the door stays where the hinge stops.
            let theta = render::radians_f64(angle).min(path.opening());
            match path.coupler(theta) {
                Some(matrix) => Placement::Turned(matrix),
                None => Placement::REST,
            }
        }
    }
}

/// The angle a pose value turns by; `None` when it doesn't turn.
fn turned(value: PoseValue, opening: Angle) -> Option<Angle> {
    let angle = match value {
        PoseValue::Open => opening,
        PoseValue::Angle(angle) => angle,
        PoseValue::Closed | PoseValue::Travel(_) => return None,
    };
    (angle.microdeg() != 0).then_some(angle)
}
