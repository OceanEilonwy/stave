//! Executable spec: mitres (the `diagonal()` cut, "Angled cuts", and std's
//! `mitre`) and edge profiles (a part's `edges:`, "Edge profiles"), on the
//! picture frame example and small projects.

mod common;

use std::path::Path;

use common::{design, only_error, open, part};
use stave_geom::solid::Solid;
use stave_lang::placement::PartAxis;
use stave_model::solved::{CutKind, CutPlace, CutSource, End, LocalFace, Part, SolvedDesign};

fn picture_frame() -> stave_app::Opened {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/picture_frame"
    ));
    stave_app::open(path).unwrap()
}

/// The angled prisms the geometry made in a part.
fn prisms(part: &Part, design: &SolvedDesign) -> Vec<(PartAxis, i128)> {
    let solid = Solid::of(part, design);
    assert!(solid.misshapen().is_empty(), "{}", part.name);
    assert!(solid.skipped().is_empty(), "{}", part.name);
    solid
        .prisms()
        .iter()
        .map(|prism| (prism.face_axis(), prism.outline().twice_area().abs()))
        .collect()
}

fn nm2(mm_a: i128, mm_b: i128) -> i128 {
    mm_a * 1_000_000 * mm_b * 1_000_000
}

#[test]
fn the_picture_frame_checks_with_its_mitres_and_profiles() {
    let opened = picture_frame();
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
    // Every member of the frame and every side of the box is mitred at
    // both ends.
    for name in [
        "rail_front",
        "rail_back",
        "stile_left",
        "stile_right",
        "box_front",
        "box_back",
        "box_left",
        "box_right",
    ] {
        let diagonals = part(design, name)
            .cuts
            .iter()
            .filter(|cut| cut.kind == CutKind::Diagonal)
            .count();
        assert_eq!(diagonals, 2, "{name}");
    }
}

#[test]
fn a_frame_member_is_mitred_across_its_width_and_a_box_side_across_its_thickness() {
    let opened = picture_frame();
    let design = design(&opened);
    // The frame lies flat: the triangle is on the face, 40mm by 40mm, cut
    // through the thickness.
    let stile = part(design, "stile_left");
    let flat: Vec<_> = prisms(stile, design)
        .into_iter()
        .filter(|(axis, _)| *axis == PartAxis::Thickness)
        .collect();
    assert!(
        flat.contains(&(PartAxis::Thickness, nm2(40, 40))),
        "{flat:?}"
    );
    // The box's sides stand on edge: the triangle is across the thickness
    // of both, 12mm by 12mm, cut through the width (its height).
    let side = part(design, "box_left");
    let standing = prisms(side, design);
    assert_eq!(
        standing
            .iter()
            .filter(|prism| **prism == (PartAxis::Width, nm2(12, 12)))
            .count(),
        2,
        "{standing:?}"
    );
}

#[test]
fn an_edge_profile_is_a_cut_of_the_part_on_the_faces_it_names() {
    let opened = picture_frame();
    let design = design(&opened);
    let rail = part(design, "rail_front");
    // Lying flat, its length right and width back: top is the end of its
    // thickness, front the start of its width, back the end.
    let top = LocalFace {
        axis: PartAxis::Thickness,
        end: End::End,
    };
    let profiles: Vec<_> = rail
        .cuts
        .iter()
        .filter(|cut| cut.source == CutSource::Edges { part: rail.id })
        .map(|cut| (cut.face, cut.place.clone(), cut.kind.clone()))
        .collect();
    assert_eq!(
        profiles,
        vec![
            (
                top,
                CutPlace::Edge {
                    other: LocalFace {
                        axis: PartAxis::Width,
                        end: End::Start
                    }
                },
                CutKind::Roundover {
                    radius: common::mm("6")
                }
            ),
            (
                top,
                CutPlace::Edge {
                    other: LocalFace {
                        axis: PartAxis::Width,
                        end: End::End
                    }
                },
                CutKind::Chamfer {
                    size: common::mm("3")
                }
            ),
        ]
    );
}

#[test]
fn a_chamfer_takes_a_triangle_off_the_whole_drawn_edge() {
    let opened = open(
        "part shelf : pine_19 {
  length: 500mm
  width:  200mm
  edges:  chamfer(3mm, on: edge(top, front))
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
joint glue : glued_face(shelf, into: base)
",
    );
    let design = design(&opened);
    let shelf = part(design, "shelf");
    assert_eq!(prisms(shelf, design), vec![(PartAxis::Length, nm2(3, 3))]);
}

#[test]
fn around_a_face_shapes_its_four_edges() {
    let opened = open(
        "part top : pine_19 {
  length: 500mm
  width:  200mm
  edges:  roundover(6mm, on: around(top))
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
joint glue : glued_face(top, into: base)
",
    );
    let design = design(&opened);
    let top = part(design, "top");
    let rounded = top
        .cuts
        .iter()
        .filter(|cut| matches!(cut.kind, CutKind::Roundover { .. }))
        .count();
    assert_eq!(rounded, 4);
    // Each is drawn as flats that touch the circle: a little more wood than
    // the curve (whose waste is r² (1 - π/4)), never less, and close to it.
    let waste = 36.0e12 * (1.0 - std::f64::consts::FRAC_PI_4);
    for (_, twice) in prisms(top, design) {
        let area = twice as f64 / 2.0;
        assert!(area <= waste && area > waste * 0.97, "{area} vs {waste}");
    }
}

#[test]
fn a_mitre_between_parts_that_run_the_same_way_is_a_type_mismatch() {
    let opened = open(
        "part strip : pine_19 {
  length: 600mm
  width:  100mm
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
joint j : mitre(strip, into: base)
",
    );
    only_error(
        &opened,
        "type-mismatch",
        "j: its parts run the same way, so they make no corner to cut on the diagonal",
    );
}

#[test]
fn a_chamfer_of_nothing_is_out_of_range() {
    let opened = open(
        "part shelf : pine_19 {
  length: 500mm
  width:  200mm
  edges:  chamfer(0mm, on: edge(top, front))
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
joint glue : glued_face(shelf, into: base)
",
    );
    only_error(
        &opened,
        "out-of-range",
        "a chamfer must be more than 0mm, found 0mm",
    );
}
