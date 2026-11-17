//! Meshes of a cup hinge's mechanism: the cup with its rim and the boss
//! pins (moving with the door), the plate, arm and knuckle with the arm
//! pins (fixed), and the two links, each drawn at rest and turned about its
//! arm pin by the viewer (see [`crate::linkage`]).
//!
//! Sizes are the catalogue's, as the hinge's kind declares them (its
//! members' shapes and its mechanism's pins): simple boxes and cylinders,
//! enough to see how the mechanism works.

use std::f32::consts::TAU;

use glam::Vec3;
use stave_model::solved::{CupMechanism, Hardware, HardwareMember, LinkBar, PlanePoint};

use crate::build::{self, Builder, Corners, Frame, GrainAxes, ProfilePoint};
use crate::{Geometry, mesh, render};

/// How many facets the cup and pins have around.
const ROUND: u16 = 32;

/// The cup's steel rim: how far it reaches past the bore, and how thick it
/// is on the door's face (millimetres).
const RIM_WIDTH: f32 = 2.5;
const RIM_THICKNESS: f32 = 0.8;
/// The cup's wall (millimetres).
const WALL: f32 = 0.8;

/// The frame of the mechanism's plane in the render frame: `x` toward the
/// free side, `y` toward the swing side, the third along the pins.
struct PlaneFrame {
    x: Vec3,
    y: Vec3,
    along: Vec3,
}

impl PlaneFrame {
    fn of(mechanism: &CupMechanism) -> PlaneFrame {
        let frame = mechanism.linkage.frame();
        let x = render::direction(frame.free_side());
        let y = render::direction(frame.swing_toward());
        PlaneFrame {
            x,
            y,
            along: x.cross(y),
        }
    }
}

/// A point of the plane, in the render frame, at the mechanism's middle
/// along the pins.
fn plane_point(mechanism: &CupMechanism, point: PlanePoint) -> Vec3 {
    render::point(mechanism.linkage.frame().world(point))
}

/// A pin: a cylinder along the pins, centred on the mechanism's middle.
fn pin(centre: Vec3, plane: &PlaneFrame, diameter: f32, length: f32) -> Geometry {
    let frame = Frame::new(
        centre - plane.along * (length / 2.0),
        [plane.along, plane.x, plane.y],
    );
    let radius = diameter / 2.0;
    build::lathe(
        &frame,
        [0.0, 0.0],
        &[
            ProfilePoint::new(0.0, radius),
            ProfilePoint::new(length, radius),
        ],
        Corners::Sharp,
        GrainAxes::LENGTH,
    )
}

/// A link at rest: a bar (or the two flanges of a strap) from pin to pin,
/// reaching past each pin by half its thickness (the collision checks use
/// the same bar).
fn link(from: Vec3, to: Vec3, plane: &PlaneFrame, bar: &LinkBar) -> Geometry {
    let direction = (to - from).normalize_or_zero();
    let across = plane.along.cross(direction).normalize_or_zero();
    let thickness = render::mm(bar.thickness);
    let length = (to - from).length() + thickness;
    let width = render::mm(bar.width);
    let slot = render::mm(bar.slot);
    let start = from - direction * (thickness / 2.0) - across * (thickness / 2.0);
    let frame = Frame::new(start, [direction, across, plane.along]);
    let piece = |low: f32, high: f32| {
        build::cuboid(
            &frame,
            Vec3::new(0.0, 0.0, low),
            Vec3::new(length, thickness, high - low),
            GrainAxes::LENGTH,
        )
    };
    if slot > 0.0 {
        mesh::merge(vec![
            piece(-width / 2.0, -slot / 2.0),
            piece(slot / 2.0, width / 2.0),
        ])
    } else {
        piece(-width / 2.0, width / 2.0)
    }
}

/// A box member (the plate, the arm, the knuckle).
fn member(member: &HardwareMember) -> Geometry {
    let frame = Frame::of_pose(&member.pose);
    build::cuboid(
        &frame,
        Vec3::ZERO,
        render::size(member.envelope.size()),
        GrainAxes::LENGTH,
    )
}

/// The cup: a thin steel cup in the bore, open toward the cabinet, with a
/// rim on the door's face. `mouth` is the middle of the bore on the face,
/// `into` points into the door, `radius` and `depth` are the bore's.
fn cup(mouth: Vec3, into: Vec3, side: Vec3, radius: f32, depth: f32) -> Geometry {
    let mut builder = Builder::default();
    let other = into.cross(side).normalize_or_zero();
    let ring = |around: u16| -> Vec3 {
        let angle = TAU * f32::from(around) / f32::from(ROUND);
        side * angle.cos() + other * angle.sin()
    };
    let inner = radius - WALL;
    let rim = radius + RIM_WIDTH;
    for around in 0..ROUND {
        let (a, b) = (ring(around), ring(around + 1));
        // Faces as quads: (corner positions, normal per corner).
        let quads: [([Vec3; 4], [Vec3; 4]); 5] = [
            // Outer wall.
            (
                [
                    mouth + a * radius,
                    mouth + b * radius,
                    mouth + b * radius + into * depth,
                    mouth + a * radius + into * depth,
                ],
                [a, b, b, a],
            ),
            // Inner wall, facing the cup's middle.
            (
                [
                    mouth + a * inner,
                    mouth + into * (depth - WALL) + a * inner,
                    mouth + into * (depth - WALL) + b * inner,
                    mouth + b * inner,
                ],
                [-a, -a, -b, -b],
            ),
            // Bottom, inside and outside.
            (
                [
                    mouth + into * (depth - WALL),
                    mouth + into * (depth - WALL) + b * inner,
                    mouth + into * (depth - WALL) + a * inner,
                    mouth + into * (depth - WALL),
                ],
                [-into; 4],
            ),
            (
                [
                    mouth + into * depth,
                    mouth + into * depth + a * radius,
                    mouth + into * depth + b * radius,
                    mouth + into * depth,
                ],
                [into; 4],
            ),
            // The rim on the door's face, facing the cabinet.
            (
                [
                    mouth - into * RIM_THICKNESS + a * inner,
                    mouth - into * RIM_THICKNESS + a * rim,
                    mouth - into * RIM_THICKNESS + b * rim,
                    mouth - into * RIM_THICKNESS + b * inner,
                ],
                [-into; 4],
            ),
        ];
        for (corners, normals) in quads {
            let index = corners
                .iter()
                .zip(normals)
                .map(|(corner, normal)| builder.vertex(*corner, normal, [0.0; 3]))
                .collect::<Vec<_>>();
            if let [p, q, r, s] = index[..] {
                builder.triangle(p, q, r);
                builder.triangle(p, r, s);
            }
        }
        // The rim's outer edge and the gap between the rim and the wall.
        let outer = [
            mouth - into * RIM_THICKNESS + a * rim,
            mouth + a * rim,
            mouth + b * rim,
            mouth - into * RIM_THICKNESS + b * rim,
        ];
        let index = outer
            .iter()
            .zip([a, a, b, b])
            .map(|(corner, normal)| builder.vertex(*corner, normal, [0.0; 3]))
            .collect::<Vec<_>>();
        if let [p, q, r, s] = index[..] {
            builder.triangle(p, q, r);
            builder.triangle(p, r, s);
        }
        let lip = [
            mouth - into * RIM_THICKNESS + a * inner,
            mouth - into * RIM_THICKNESS + b * inner,
            mouth + b * inner,
            mouth + a * inner,
        ];
        let index = lip
            .iter()
            .zip([-a, -b, -b, -a])
            .map(|(corner, normal)| builder.vertex(*corner, normal, [0.0; 3]))
            .collect::<Vec<_>>();
        if let [p, q, r, s] = index[..] {
            builder.triangle(p, q, r);
            builder.triangle(p, r, s);
        }
        builder.edge(
            mouth - into * RIM_THICKNESS + a * rim,
            mouth - into * RIM_THICKNESS + b * rim,
        );
        builder.edge(
            mouth + into * depth + a * radius,
            mouth + into * depth + b * radius,
        );
    }
    builder.finish()
}

/// The parts of a cup hinge's mechanism, at rest.
#[derive(Debug, Clone, PartialEq)]
pub struct MechanismGeometry {
    /// The cup with its rim and the boss pins: moves with the door.
    pub cup: Geometry,
    /// The plate, the arm and its knuckle, and the arm pins: fixed.
    pub fixed: Geometry,
    /// Each link, in the order of the linkage's links: turns about its
    /// arm pin.
    pub links: [Geometry; 2],
}

/// The meshes of a cup hinge's mechanism; `None` for any other hardware.
pub fn mechanism(hardware: &Hardware) -> Option<MechanismGeometry> {
    let mechanism = hardware.mechanism.as_ref()?;
    let plane = PlaneFrame::of(mechanism);
    let pin_diameter = render::mm(mechanism.pin_diameter);
    let pin_length = render::mm(mechanism.pin_length);
    let links = mechanism.linkage.links();
    // The cup: its mouth on the mount face, centred on the bore.
    let size = hardware.envelope.size();
    let frame = Frame::of_pose(&hardware.pose);
    let radius = render::mm(size.length) / 2.0;
    let depth = render::mm(size.thickness);
    let into = frame.vector(Vec3::Z);
    let mouth = frame.point(Vec3::new(radius, radius, 0.0));
    let mut cup_pieces = vec![cup(mouth, into, plane.x, radius, depth)];
    let mut fixed_pieces = Vec::new();
    if let Some(plate) = &hardware.fixed {
        fixed_pieces.push(member(plate));
    }
    fixed_pieces.push(member(&mechanism.arm));
    fixed_pieces.push(member(&mechanism.knuckle));
    let mut link_meshes = Vec::with_capacity(2);
    for (link_rest, bar) in links.iter().zip(&mechanism.links) {
        let ground = plane_point(mechanism, link_rest.ground);
        let coupler = plane_point(mechanism, link_rest.coupler);
        fixed_pieces.push(pin(ground, &plane, pin_diameter, pin_length));
        cup_pieces.push(pin(coupler, &plane, pin_diameter, pin_length));
        link_meshes.push(link(ground, coupler, &plane, bar));
    }
    let [first, second]: [Geometry; 2] = link_meshes.try_into().ok()?;
    Some(MechanismGeometry {
        cup: mesh::merge(cup_pieces),
        fixed: mesh::merge(fixed_pieces),
        links: [first, second],
    })
}
