//! Executable spec: slanted members ("Slanted members", "Shapes"): a
//! cylinder turned by `tilt:` toward a frame direction, the holes a drill
//! makes along its slant (`from_depth:` included), and std's
//! `pocket_screw`, on the face frame example.

mod common;

use std::path::Path;

use common::{design, part};
use stave_geom::collide::WorldSolid;
use stave_geom::solid::Solid;
use stave_model::solved::{CutKind, Depth, MemberShape, SolvedDesign};

fn face_frame() -> stave_app::Opened {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/face_frame"
    ));
    stave_app::open(path).unwrap()
}

/// The distance from a point to the line through two others, in
/// nanometres.
fn off_line(point: glam::DVec3, a: glam::DVec3, b: glam::DVec3) -> f64 {
    (point - a).reject_from((b - a).normalize()).length()
}

fn vector(point: stave_model::solved::Point) -> glam::DVec3 {
    glam::DVec3::new(
        point.across.nm() as f64,
        point.up.nm() as f64,
        point.deep.nm() as f64,
    )
}

/// The slanted screws of the design, each as the two ends of its axis.
fn screws(design: &SolvedDesign) -> Vec<(glam::DVec3, glam::DVec3)> {
    design
        .joints()
        .iter()
        .flat_map(|joint| joint.pairs.iter())
        .flat_map(|pair| pair.members.iter())
        .flat_map(|member| member.shapes().iter())
        .filter_map(MemberShape::slanted_ends)
        .map(|(a, b)| (vector(a), vector(b)))
        .collect()
}

#[test]
fn the_face_frame_checks_with_its_pocket_screws() {
    let opened = face_frame();
    assert!(
        opened.diagnostics.is_empty(),
        "{:?}",
        opened
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let design = design(&opened);
    // Two screws at each of the four corners, slanted 15deg.
    let found = screws(design);
    assert_eq!(found.len(), 8);
    for joint in design.joints() {
        for pair in &joint.pairs {
            for member in &pair.members {
                for shape in member.shapes() {
                    let MemberShape::Slanted { angle, length, .. } = shape else {
                        panic!("a slanted screw: {shape:?}");
                    };
                    assert_eq!(angle.to_string(), "15deg");
                    assert_eq!(*length, common::mm("32"));
                }
            }
        }
    }
}

#[test]
fn every_hole_runs_along_the_axis_of_its_screw() {
    let opened = face_frame();
    let design = design(&opened);
    let found = screws(design);
    for name in ["stile_l", "stile_r", "rail_top", "rail_bottom"] {
        let host = part(design, name);
        let world = WorldSolid::of_part(host, &Solid::of(host, design));
        assert!(world.holes().is_empty(), "{name} has only slanted holes");
        assert!(!world.slant_holes().is_empty(), "{name} is drilled");
        for hole in world.slant_holes() {
            let (c, d) = (vector(hole.from), vector(hole.to));
            // Some screw's axis runs through both ends of the hole, to
            // within a few nanometres of rounding.
            assert!(
                found
                    .iter()
                    .any(|(a, b)| off_line(c, *a, *b) < 10.0 && off_line(d, *a, *b) < 10.0),
                "a hole in {name} off every screw: {hole:?}"
            );
        }
    }
}

#[test]
fn the_pocket_starts_below_the_end_and_runs_out_through_the_face() {
    let opened = face_frame();
    let design = design(&opened);
    let rail = part(design, "rail_top");
    let pockets: Vec<_> = rail
        .cuts
        .iter()
        .filter(|cut| cut.name.as_ref().is_some_and(|n| n.as_str() == "pocket"))
        .collect();
    assert_eq!(pockets.len(), 4, "two screws at each end");
    for pocket in pockets {
        let CutKind::Drill {
            diameter,
            depth,
            from_depth,
            tilt,
        } = &pocket.kind
        else {
            panic!("a drill: {:?}", pocket.kind);
        };
        assert_eq!(*diameter, common::mm("9.5"));
        assert_eq!(*depth, Depth::Through);
        assert_eq!(*from_depth, common::mm("20"));
        let tilt = tilt.expect("the pocket follows the screw's slant");
        assert_eq!(tilt.angle.to_string(), "15deg");
        // Drilled from the end of the rail, leaning toward a face.
        assert_eq!(pocket.face.axis, stave_lang::placement::PartAxis::Length);
        assert_eq!(tilt.toward.axis, stave_lang::placement::PartAxis::Thickness);
    }
}

#[test]
fn a_rail_with_pockets_is_drawn_with_them() {
    let opened = face_frame();
    let design = design(&opened);
    let rail = part(design, "rail_top");
    let drilled = stave_geom::cut_part(rail, &Solid::of(rail, design));
    let mut plain_rail = rail.clone();
    plain_rail.cuts.clear();
    let plain = stave_geom::cut_part(&plain_rail, &Solid::of(&plain_rail, design));
    assert!(
        drilled.mesh.vertices.len() > plain.mesh.vertices.len(),
        "the holes add to the mesh"
    );
}

#[test]
fn a_screw_past_the_bottom_of_its_slanted_hole_is_an_overlap() {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/broken/slanted_screw_past_its_hole"
    ));
    let opened = stave_app::open(path).unwrap();
    let errors: Vec<_> = opened
        .diagnostics
        .iter()
        .filter(|d| d.severity == stave_lang::Severity::Error)
        .collect();
    let [error] = errors.as_slice() else {
        panic!("one error: {errors:?}");
    };
    assert_eq!(error.message, "corner's screw and stile overlap");
    assert_eq!(
        error.help.as_deref(),
        Some(
            "the screw of 'corner' goes 5mm past the bottom of its hole in stile; make the hole as deep as the screw reaches, or the screw 5mm shorter"
        )
    );
}
